//! Message sending orchestration — send_message_flow and send_tool_response_flow.

use super::models::{TaskMessage, TaskMessageContentBlock, TaskMessageContext};
use crate::message::models::{ContentBlockType, MessageRole, MessageType};
use crate::shared_models::AgentMode;
use crate::state::AppState;
use crate::task::dao::TaskPatch;
use crate::task::schemas::{FileContextData, MessageContextInput};

use super::contexts::message_context_input_to_row;
use super::enricher;
use super::error::MessageError;

/// Whether the task exists and is in `Closed` status. Fail-open on DB
/// errors: a DB blip must not silently drop a legitimate message.
async fn task_is_closed(state: &AppState, task_id: &str) -> bool {
    match state.tasks.find_by_id_internal(task_id).await {
        Ok(Some(task)) => task.status == crate::task::models::TaskStatus::Closed,
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(
                task_id = %task_id,
                error = %e,
                "Failed to fetch task status for closed-task guard; proceeding"
            );
            false
        }
    }
}

/// The main message flow — called by message handlers, task creation, and external inbound.
#[allow(clippy::too_many_arguments)]
pub async fn send_message_flow(
    state: &AppState,
    task_id: &str,
    message: &str,
    model: Option<&str>,
    agent_mode: Option<&AgentMode>,
    file_ids: &[String],
    contexts: &[MessageContextInput],
    resume: bool,
    external_event_id: Option<&str>,
    external_adapter: Option<&str>,
    external_subject_key: Option<&str>,
    external_source_name: Option<&str>,
    source_task_id: Option<&str>,
    start_fresh: bool,
    sent_by: Option<&str>,
) -> Result<Option<String>, MessageError> {
    // A closed task is terminal (no reopen path) so no send may persist a
    // message or wake the agent. Inbound external messages are dropped
    // silently — backstop for the close-vs-comment race; steady state,
    // inactive bindings already keep these out of routing. Internal/API sends
    // surface a 409 so the caller knows nothing was queued. `Error` tasks
    // stay sendable to match `start_task` recovery semantics.
    if task_is_closed(state, task_id).await {
        if let Some(adapter) = external_adapter {
            tracing::debug!(
                task_id = %task_id,
                external_adapter = %adapter,
                "Dropping inbound external message for closed task"
            );
            return Ok(None);
        }
        return Err(MessageError::TaskClosed(task_id.to_string()));
    }

    // Dedup: if this external message was already persisted (or was just sent
    // outbound and is racing the webhook), skip the insert entirely. Prevents
    // duplicates from Slack retries, re-delivered webhooks, and `moat reply`
    // self-echoes.
    if let Some(ext_id) = external_event_id {
        // 1. In-memory race-window check — catches webhooks that land before
        //    `set_external_event_id` has committed.
        if state.recently_sent.contains(task_id, ext_id) {
            tracing::debug!(
                task_id = %task_id,
                external_event_id = %ext_id,
                "Filtered self-echo via recently_sent cache"
            );
            return Ok(None);
        }

        // 2. DB check — tolerates both new prefixed format and any legacy
        //    bare-int rows written before the prefix scheme.
        let candidates = external_adapter
            .and_then(|provider_name| {
                state
                    .adapters
                    .as_ref()
                    .and_then(|providers| providers.get_by_name(provider_name))
            })
            .map_or_else(
                || vec![ext_id.to_string()],
                |provider| provider.external_id_aliases(ext_id),
            );
        let exists = state
            .messages
            .has_any_external_event_id(task_id, &candidates)
            .await
            .map_err(|e| MessageError::Internal(format!("Dedup check failed: {e}")))?;
        if exists {
            tracing::debug!(task_id = %task_id, external_event_id = %ext_id, "Duplicate external message, skipping");
            return Ok(None);
        }
    }

    let effective_message = message.to_string();

    // Bake attribution into message text so the agent sees it via the normal content chain.
    let attributed_message = match source_task_id {
        Some(id) => format!(
            "Message from task `{id}`:\n\n{effective_message}\n\n\
             To reply, run: `moat send --task {id} --text \"<your reply>\"`"
        ),
        None => effective_message.clone(),
    };

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    // Build enriched content blocks (includes preview, element contexts + file attachments)
    let enriched_blocks = enricher::build_content_blocks(
        state.files.as_ref(),
        &state.config,
        &attributed_message,
        contexts,
        file_ids,
    )
    .await;
    let content = serde_json::to_value(&enriched_blocks)
        .unwrap_or_else(|_| serde_json::json!([{ "type": "text", "text": attributed_message }]));

    // Persist the attached file IDs so the sandbox can fetch them via gRPC
    // before invoking the agent.
    let attachments = if file_ids.is_empty() {
        None
    } else {
        Some(serde_json::json!({ "file_ids": file_ids }))
    };

    let msg_row = TaskMessage {
        id: 0, // will be assigned by DB
        task_id: task_id.to_string(),
        uuid: msg_uuid,
        message_type: MessageType::User,
        user_type: "human".to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id: None,
        // Queued user rows carry no turn until the sandbox acknowledges them:
        // the run that consumes them decides which turn they belong to.
        turn_number: None,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: now.clone(),
        message_role: MessageRole::User,
        message_content: content,
        assistant_message_id: None,
        assistant_message_type: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        assistant_usage: None,
        tool_use_result: None,
        sdk_metadata: None,
        attachments,
        agent_session_id: None, // Not yet acknowledged
        will_not_resume: None,
        run_metrics: None,
        agent_mode: agent_mode.cloned(),
        model: model.map(std::string::ToString::to_string),
        resume,
        sent_at: None,
        created_at: None,
        external_event_id: external_event_id.map(std::string::ToString::to_string),
        external_adapter: external_adapter.map(std::string::ToString::to_string),
        external_subject_key: external_subject_key.map(std::string::ToString::to_string),
        external_source_name: external_source_name.map(std::string::ToString::to_string),
        source_task_id: source_task_id.map(std::string::ToString::to_string),
        start_fresh,
        sent_by: sent_by.map(std::string::ToString::to_string),
    };

    // Build content blocks for the showInUI=true user text
    let ui_block = TaskMessageContentBlock {
        id: 0,
        task_message_id: 0, // will be set by DAO
        sort_order: 0,
        block_type: ContentBlockType::Text,
        show_in_ui: true,
        text_content: Some(effective_message),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    };

    // Persist contexts: sender-supplied (preview/element) + synthesised
    // file contexts so the read path can surface uploaded files alongside the
    // other context items. The `attachments` JSON column is still written
    // above for the sandbox gRPC pre-fetch path.
    let mut all_contexts: Vec<MessageContextInput> = contexts.to_vec();
    for file_id in file_ids {
        let data = match state.files.find_by_id(file_id).await {
            Ok(Some(meta)) => FileContextData {
                filename: meta.filename,
                mime_type: meta.mime_type,
                size: meta.size,
            },
            Ok(None) => {
                tracing::warn!(
                    file_id,
                    "File metadata not found when synthesising File context"
                );
                FileContextData::default()
            }
            Err(e) => {
                tracing::warn!(file_id, error = %e, "Failed to load file metadata for File context");
                FileContextData::default()
            }
        };
        all_contexts.push(MessageContextInput::File {
            reference_id: file_id.clone(),
            data,
        });
    }

    let context_rows: Vec<TaskMessageContext> = all_contexts
        .iter()
        .map(message_context_input_to_row)
        .collect();

    state
        .messages
        .append(&msg_row, &context_rows, &[ui_block])
        .await
        .map_err(|e| MessageError::Internal(format!("Failed to persist message: {e}")))?;

    tracing::info!(task_id = %task_id, "Message persisted");

    // Advance the conversation clock. The persisted pending row is the
    // reconciler's durable wake input after idle/user stops; sandbox intent is
    // written only by lifecycle (through the ensure-and-notify start path),
    // never here.
    state
        .tasks
        .update_task(
            task_id,
            &TaskPatch {
                last_message_at: Some(now),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| MessageError::Internal(format!("Failed to update task: {e}")))?;

    Ok(Some(msg_row.uuid))
}

/// Persist an error result message visible in the chat.
///
/// Used when the message was saved but a subsequent operation (e.g. sandbox start) failed.
/// The error text is prefixed to make it clear to both the UI and the agent that this
/// is a system/harness error, not user input.
/// Fire-and-forget: logs on failure but never propagates errors.
pub async fn persist_error_result(state: &AppState, task_id: &str, error_text: &str) {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    let content = serde_json::json!([{ "type": "text", "text": error_text }]);

    let turn_number = match state.task_turns.latest_turn_number(task_id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(task_id, error = %e, "Failed to read current turn number for error result");
            None
        }
    };

    let msg_row = TaskMessage {
        id: 0,
        task_id: task_id.to_string(),
        uuid: msg_uuid,
        message_type: MessageType::Error,
        user_type: "system".to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id: None,
        turn_number,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: now,
        message_role: MessageRole::Assistant,
        message_content: content.clone(),
        assistant_message_id: None,
        assistant_message_type: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        assistant_usage: None,
        tool_use_result: None,
        sdk_metadata: None,
        attachments: None,
        agent_session_id: None,
        will_not_resume: Some(false),
        run_metrics: None,
        agent_mode: None,
        model: None,
        resume: true,
        sent_at: None,
        created_at: None,
        external_event_id: None,
        external_adapter: None,
        external_subject_key: None,
        external_source_name: None,
        source_task_id: None,
        start_fresh: false,
        sent_by: None,
    };

    let blocks = vec![TaskMessageContentBlock {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: ContentBlockType::Text,
        show_in_ui: true,
        text_content: Some(error_text.to_string()),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    }];

    if let Err(e) = state.messages.append(&msg_row, &[], &blocks).await {
        tracing::error!(task_id = %task_id, error = %e, "Failed to persist error result message");
    }
}

/// Send a tool response (AskUserQuestion / ExitPlanMode).
pub async fn send_tool_response_flow(
    state: &AppState,
    task_id: &str,
    tool_use_id: &str,
    _tool_name: &str,
    tool_response: Option<&serde_json::Value>,
    sent_by: Option<&str>,
) -> Result<(), MessageError> {
    // Same closed-task guard as `send_message_flow`. Tool responses only
    // arrive via the HTTP handler, so a closed task always surfaces the 409.
    if task_is_closed(state, task_id).await {
        return Err(MessageError::TaskClosed(task_id.to_string()));
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    let response_text = tool_response
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .unwrap_or_default();

    let content = serde_json::json!([{
        "type": "tool_result",
        "tool_use_id": tool_use_id,
        "content": response_text
    }]);

    let msg_row = TaskMessage {
        id: 0,
        task_id: task_id.to_string(),
        uuid: msg_uuid,
        message_type: MessageType::User,
        user_type: "human".to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id: None,
        // Queued user rows carry no turn until the sandbox acknowledges them:
        // the run that consumes them decides which turn they belong to.
        turn_number: None,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: now,
        message_role: MessageRole::User,
        message_content: content,
        assistant_message_id: None,
        assistant_message_type: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        assistant_usage: None,
        tool_use_result: None,
        sdk_metadata: None,
        attachments: None,
        agent_session_id: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode: None,
        model: None,
        resume: true,
        sent_at: None,
        created_at: None,
        external_event_id: None,
        external_adapter: None,
        external_subject_key: None,
        external_source_name: None,
        source_task_id: None,
        start_fresh: false,
        sent_by: sent_by.map(std::string::ToString::to_string),
    };

    let contexts: Vec<TaskMessageContext> = vec![];
    let blocks: Vec<TaskMessageContentBlock> = vec![];
    state
        .messages
        .append(&msg_row, &contexts, &blocks)
        .await
        .map_err(|e| MessageError::Internal(format!("Failed to persist tool response: {e}")))?;

    tracing::info!(task_id = %task_id, tool_use_id = %tool_use_id, "Tool response sent");

    Ok(())
}
