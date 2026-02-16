//! Message sending orchestration — send_message_flow and send_tool_response_flow.

use super::models::{TaskMessageContentBlock, TaskMessageContext, TaskMessage};
use crate::task::dao::TaskPatch;
use crate::message::models::{ContentBlockType, MessageRole, MessageType};
use crate::state::AppState;
use crate::task::schemas::MessageContextInput;

use super::enricher;
use super::error::MessageError;

/// The main message flow — called by message handlers, task creation, and channel inbound.
pub async fn send_message_flow(
    state: &AppState,
    task_id: &str,
    message: &str,
    model: Option<&str>,
    agent_mode: Option<&str>,
    file_ids: &[String],
    contexts: &[MessageContextInput],
) -> Result<(), MessageError> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    // Build enriched content blocks (includes figma, preview, element contexts + file attachments)
    let enriched_blocks =
        enricher::build_content_blocks(
            state.figma_imports.as_ref(),
            state.files.as_ref(),
            &state.config,
            message,
            contexts,
            file_ids,
        )
        .await;
    let content = serde_json::to_value(&enriched_blocks).unwrap_or_else(|_| {
        serde_json::json!([{ "type": "text", "text": message }])
    });

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
        tool_use_result: None,
        attachments: None,
        agent_session_id: None, // Not yet acknowledged
        turn_status: None,
        turn_result: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode: agent_mode.map(|s| s.to_string()),
        model: model.map(|s| s.to_string()),
        resume_session_id: None,
        sent_at: None,
        created_at: None,
    };

    // Build content blocks for the showInUI=true user text
    let ui_block = TaskMessageContentBlock {
        id: 0,
        task_message_id: 0, // will be set by DAO
        sort_order: 0,
        block_type: ContentBlockType::Text,
        show_in_ui: true,
        text_content: Some(message.to_string()),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    };

    // Persist message
    let contexts: Vec<TaskMessageContext> = vec![];
    state.messages.append(&msg_row, &contexts, &[ui_block])
        .await
        .map_err(|e| MessageError::Internal(format!("Failed to persist message: {e}")))?;

    tracing::info!(task_id = %task_id, "Message persisted");

    // Update task last_message_at (running state is derived on frontend)
    state.tasks.update_task(
        task_id,
        &TaskPatch {
            last_message_at: Some(now),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| MessageError::Internal(format!("Failed to update task: {e}")))?;

    Ok(())
}

/// Send a tool response (AskUserQuestion / ExitPlanMode).
pub async fn send_tool_response_flow(
    state: &AppState,
    task_id: &str,
    tool_use_id: &str,
    _tool_name: &str,
    tool_response: Option<&serde_json::Value>,
) -> Result<(), MessageError> {
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
        tool_use_result: None,
        attachments: None,
        agent_session_id: None,
        turn_status: None,
        turn_result: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode: None,
        model: None,
        resume_session_id: None,
        sent_at: None,
        created_at: None,
    };

    let contexts: Vec<TaskMessageContext> = vec![];
    let blocks: Vec<TaskMessageContentBlock> = vec![];
    state.messages.append(&msg_row, &contexts, &blocks)
        .await
        .map_err(|e| MessageError::Internal(format!("Failed to persist tool response: {e}")))?;

    tracing::info!(task_id = %task_id, tool_use_id = %tool_use_id, "Tool response sent");

    Ok(())
}
