//! Agent event handlers — process agent.completed, agent.stopped, agent.error events.

use moatless_events::{TaskSubType, TaskTurnSubType};

use crate::message::models::{ContentBlockType, MessageRole, MessageType};
use crate::message::models::{TaskMessage, TaskMessageContentBlock, TaskMessageContext};
use crate::shared_models::AgentMode;
use crate::state::AppState;
use crate::task::dao::TaskPatch;
use crate::task::models::TaskStatus;

use super::models::{AgentEvent, SdkResultMessage};

/// Derive a deterministic uuid for a persisted message from the event id.
///
/// Dedup happens at the DB layer via `task_messages.uuid UNIQUE` +
/// `ON CONFLICT (uuid) DO UPDATE`. Re-delivery of the same event upserts into
/// the same row instead of creating a duplicate.
fn message_uuid_for_event(event_id: &str) -> String {
    format!("evt-{event_id}")
}

/// Handle `agent.completed` event.
///
/// The final assistant message already carries the response text and usage,
/// so we no longer persist a separate result row here. We update task status /
/// cost from the SDK result, emit follow-up events, and finalize any active
/// Linear agent session by posting a `response` activity carrying the body of
/// the last assistant message (per
/// <https://linear.app/developers/agent-interaction#activity-content-payload>).
/// If the payload fails to parse there's nothing to act on — skip.
pub async fn handle_agent_completed(state: &AppState, event: &AgentEvent) {
    let sdk_result: SdkResultMessage = match serde_json::from_value(event.payload.clone()) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                task_id = %event.task_id,
                error = %e,
                "Failed to parse agent.completed payload — skipping"
            );
            return;
        }
    };

    // Defensive safety net: if the sandbox publishes an `is_error: true` result
    // under `agent.completed` (older code paths or future agent runtimes), route
    // it through the same error-message machinery as `handle_agent_error` so the
    // failure is visible in chat instead of disappearing into status flips.
    if sdk_result.is_error.unwrap_or(false) {
        handle_agent_error(state, event).await;
        return;
    }

    let task_status = TaskStatus::Open;

    // Fallback: set real Claude Code session_id on user messages
    // (in case session_init event was missed or arrived before messages were fetched)
    if let Some(ref sid) = sdk_result.session_id {
        if let Err(e) = state
            .messages
            .set_session_id_for_task(&event.task_id, sid)
            .await
        {
            tracing::warn!(task_id = %event.task_id, error = %e, "Failed to set session_id on user messages (fallback)");
        }
    }

    // Update task
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut patch = TaskPatch {
        status: Some(task_status),
        last_message_at: Some(now),
        ..Default::default()
    };

    if let Some(cost) = sdk_result.total_cost_usd {
        patch.total_cost_usd_increment = Some(cost);
    }

    if let Err(e) = state.tasks.update_task(&event.task_id, &patch).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task after completion");
    }

    // Finalize any active Linear agent session bound to this task by posting a
    // terminal `response` activity with the last assistant message body. The
    // error path is handled by `handle_agent_error`, which already emits an
    // `error` activity, so this branch is success-only.
    notify_linear_response_for_completion(state, &event.task_id).await;

    // Close the Linear turn gate as the last step so post-turn stragglers
    // (Sandbox(Stopped), late tool_results) don't pile up in Linear.
    state.linear_active_turns.close(&event.task_id);

    // Emit events. Error completions short-circuit through `handle_agent_error`
    // above, so this path is success-only.
    if let Some(ref events) = state.events {
        events
            .publish_task_turn_event(
                &event.task_id,
                TaskTurnSubType::Completed,
                event.agent_type.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;

        let task_sub = match task_status {
            TaskStatus::Open => TaskSubType::Open,
            TaskStatus::Closed => TaskSubType::Closed,
            TaskStatus::Error => TaskSubType::Updated,
        };
        events
            .publish_task_event(
                &event.task_id,
                task_sub,
                event.agent_type.as_deref(),
                serde_json::json!({}),
            )
            .await;
    }
}

/// Resolve the last assistant message for `task_id` and emit it as a Linear
/// `response` activity. Fire-and-forget — any failure is logged and swallowed
/// so the event-handler hot path stays unblocked.
async fn notify_linear_response_for_completion(state: &AppState, task_id: &str) {
    let msg = match state.messages.get_last_assistant_message(task_id).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            tracing::warn!(
                task_id,
                "agent.completed: no assistant message found; skipping Linear response activity"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                task_id,
                error = %e,
                "agent.completed: failed to fetch last assistant message; skipping Linear response activity"
            );
            return;
        }
    };

    let body = collect_assistant_text(&msg.message_content);
    if body.trim().is_empty() {
        tracing::warn!(
            task_id,
            message_uuid = %msg.uuid,
            "agent.completed: last assistant message has no text content; skipping Linear response activity"
        );
        return;
    }

    tokio::spawn(crate::adapters::linear::agent_runtime::notify_linear_status(
        state.clone(),
        task_id.to_string(),
        crate::adapters::linear::agent_api::ActivityKind::Response,
        body,
        false,
    ));
}

/// Concatenate every `text` block from a `message_content` JSON array into a
/// single string separated by blank lines. Returns an empty string when the
/// shape is unexpected or no text blocks are present.
fn collect_assistant_text(content: &serde_json::Value) -> String {
    let Some(arr) = content.as_array() else {
        return String::new();
    };
    let mut out = String::new();
    for block in arr {
        if block.get("type").and_then(|v| v.as_str()) != Some("text") {
            continue;
        }
        let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
            continue;
        };
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(text);
    }
    out
}

/// Handle `agent.session_init` event — Claude Code session started.
///
/// Per-message acknowledgment is now handled by the sandbox via
/// `AcknowledgeMessages` gRPC call. This handler only logs the event.
pub async fn handle_agent_session_init(_state: &AppState, event: &AgentEvent) {
    let session_id = if let Some(sid) = event.payload.get("session_id").and_then(|v| v.as_str()) {
        sid
    } else {
        tracing::warn!(task_id = %event.task_id, "agent.session_init missing session_id");
        return;
    };

    tracing::info!(
        task_id = %event.task_id,
        session_id = %session_id,
        "Agent session initialized"
    );
}

/// Handle `agent.stopped` event — user requested stop.
pub async fn handle_agent_stopped(state: &AppState, event: &AgentEvent) {
    // Idempotency: drop duplicate stop fan-out. Both the sandbox and the
    // /stop HTTP handler can publish `agent.stopped` for a single user
    // action, and a hammered /stop endpoint must never accumulate phantom
    // rows. If the most recent message is already a stopped marker, this
    // event has nothing to add.
    match state.messages.last_is_stopped_marker(&event.task_id).await {
        Ok(true) => {
            tracing::debug!(
                task_id = %event.task_id,
                event_id = %event.event_id,
                "Dropping duplicate agent.stopped — task already stopped"
            );
            return;
        }
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(task_id = %event.task_id, error = %e, "Failed to check last message; proceeding with stop append");
        }
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = message_uuid_for_event(&event.event_id);

    let content = serde_json::json!([{
        "type": "text",
        "text": "Stopped by user request"
    }]);

    // Inherit agent_mode from the last user message
    let agent_mode = match state.messages.get_last_user_message(&event.task_id).await {
        Ok(Some(user_msg)) => user_msg.agent_mode,
        _ => None,
    };

    let turn_number = resolve_turn_number(state, &event.task_id, event.turn_number).await;

    let msg_row = build_result_message_row(
        &event.task_id,
        &msg_uuid,
        &now,
        content,
        MessageType::Result,
        true,
        None,
        agent_mode,
        None,
        turn_number,
    );
    let contexts: Vec<TaskMessageContext> = vec![];
    let blocks = vec![TaskMessageContentBlock {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: ContentBlockType::Text,
        show_in_ui: true,
        text_content: Some("Stopped by user request".to_string()),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    }];

    if let Err(e) = state.messages.append(&msg_row, &contexts, &blocks).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to append stopped message");
    }

    if let Err(e) = state
        .tasks
        .update_task(
            &event.task_id,
            &TaskPatch {
                status: Some(TaskStatus::Open),
                last_message_at: Some(now),
                ..Default::default()
            },
        )
        .await
    {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task after stop");
    }

    if let Some(ref events) = state.events {
        events
            .publish_task_turn_event(
                &event.task_id,
                TaskTurnSubType::Stopped,
                event.agent_type.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;
    }

    // Close the Linear turn gate — user-initiated stops don't emit a final
    // activity, so this just prevents post-stop stragglers from leaking.
    state.linear_active_turns.close(&event.task_id);
}

/// Handle `agent.error` or `sandbox.fatal` event.
pub async fn handle_agent_error(state: &AppState, event: &AgentEvent) {
    let error_msg = event
        .payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown agent error");

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = message_uuid_for_event(&event.event_id);

    let content = serde_json::json!([{
        "type": "text",
        "text": error_msg
    }]);

    // Inherit agent_mode from the last user message
    let agent_mode = match state.messages.get_last_user_message(&event.task_id).await {
        Ok(Some(user_msg)) => user_msg.agent_mode,
        _ => None,
    };

    let turn_number = resolve_turn_number(state, &event.task_id, event.turn_number).await;

    // will_not_resume=false: harness-level errors (sandbox crash, pod eviction)
    // should not prevent session resumption. The Claude session is still valid.
    let msg_row = build_result_message_row(
        &event.task_id,
        &msg_uuid,
        &now,
        content,
        MessageType::Error,
        false,
        None,
        agent_mode,
        None,
        turn_number,
    );
    let contexts: Vec<TaskMessageContext> = vec![];
    let blocks = vec![TaskMessageContentBlock {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: ContentBlockType::Text,
        show_in_ui: true,
        text_content: Some(error_msg.to_string()),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    }];

    if let Err(e) = state.messages.append(&msg_row, &contexts, &blocks).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to append error message");
    }

    if let Err(e) = state
        .tasks
        .update_task(
            &event.task_id,
            &TaskPatch {
                status: Some(TaskStatus::Open),
                last_message_at: Some(now),
                ..Default::default()
            },
        )
        .await
    {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task after error");
    }

    if let Some(ref events) = state.events {
        // Include message_id in payload for outbound integration tracking
        let mut turn_payload = event.payload.clone();
        if let Some(obj) = turn_payload.as_object_mut() {
            obj.insert("message_id".to_string(), serde_json::json!(msg_uuid));
        }

        events
            .publish_task_turn_event(
                &event.task_id,
                TaskTurnSubType::Error,
                event.agent_type.as_deref(),
                event.turn_number,
                turn_payload,
            )
            .await;
    }

    // Close the Linear turn gate after the consumer's adjacent
    // notify_linear_status(Error) spawn has been issued. Also covers
    // `handle_sandbox_fatal` transitively.
    state.linear_active_turns.close(&event.task_id);
}

/// Handle `sandbox.fatal` event — delegates to `handle_agent_error` to create
/// an error result message when the sandbox crashes fatally (OOM, crash loop, etc.).
pub async fn handle_sandbox_fatal(state: &AppState, event: &AgentEvent) {
    handle_agent_error(state, event).await;
}

/// Handle `agent.session_context_lost` event — a session resume failed because
/// the transcript was unavailable (e.g. stale session id after a pod restart)
/// and the agent fell back to a fresh session.
///
/// Persists an inline `MessageType::Error` row so the context loss is clearly
/// visible in the conversation history. Unlike [`handle_agent_error`], the run
/// CONTINUES (the fresh session is starting), so this must not flip the task
/// status, publish `task_turn.error`, or close the Linear turn gate.
pub async fn handle_agent_session_context_lost(state: &AppState, event: &AgentEvent) {
    let resume_error = event
        .payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown resume error");

    let notice = format!(
        "Previous session could not be resumed — conversation context was lost. \
         Continuing with a fresh session.\n\nResume error: {resume_error}"
    );

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = message_uuid_for_event(&event.event_id);

    let content = serde_json::json!([{
        "type": "text",
        "text": notice
    }]);

    // Inherit agent_mode from the last user message (mirrors handle_agent_error)
    let agent_mode = match state.messages.get_last_user_message(&event.task_id).await {
        Ok(Some(user_msg)) => user_msg.agent_mode,
        _ => None,
    };

    let turn_number = resolve_turn_number(state, &event.task_id, event.turn_number).await;

    // will_not_resume=false: the fresh session is valid and running.
    let msg_row = build_result_message_row(
        &event.task_id,
        &msg_uuid,
        &now,
        content,
        MessageType::Error,
        false,
        None,
        agent_mode,
        None,
        turn_number,
    );
    let contexts: Vec<TaskMessageContext> = vec![];
    let blocks = vec![TaskMessageContentBlock {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: ContentBlockType::Text,
        show_in_ui: true,
        text_content: Some(notice.clone()),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    }];

    if let Err(e) = state.messages.append(&msg_row, &contexts, &blocks).await {
        tracing::warn!(
            task_id = %event.task_id,
            error = %e,
            "Failed to append session-context-lost notice"
        );
    }
}

/// Handle `agent.message` event — persist SDK messages and auto-tag skills.
///
/// Persists both assistant messages (text + tool_use blocks) and SDK user messages
/// (tool_result blocks) to the database. Then extracts skill tags from assistant messages.
/// Matches legacy Node.js `MessageService.saveAgentMessage()` behavior.
pub async fn handle_agent_message(state: &AppState, event: &AgentEvent) {
    // 1. Determine message type from payload role
    //    SDK assistant → stored as "assistant"
    //    SDK user (tool results) → stored as "system" (distinguishes from frontend user messages)
    let msg_role = event
        .payload
        .get("message")
        .and_then(|m| m.get("role"))
        .and_then(|v| v.as_str())
        .unwrap_or("assistant");

    let (message_type, message_role) = match msg_role {
        "user" => (MessageType::System, MessageRole::User),
        _ => (MessageType::Assistant, MessageRole::Assistant),
    };

    // 2. Extract fields from payload
    let uuid = event
        .payload
        .get("uuid")
        .and_then(|v| v.as_str())
        .unwrap_or(&event.event_id);

    let session_id = event
        .payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let parent_uuid = event
        .payload
        .get("parent_tool_use_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let content = event
        .payload
        .get("message")
        .and_then(|m| m.get("content"))
        .cloned()
        .unwrap_or(serde_json::json!([]));

    // Normalize content to array if it's a string (safety net)
    let content = if content.is_string() {
        serde_json::json!([{ "type": "text", "text": content.as_str().unwrap_or("") }])
    } else {
        content
    };

    let usage = event.payload.get("message").and_then(|m| m.get("usage"));

    // 3. Inherit agent_mode from the last user message (same pattern as result messages)
    let agent_mode = match state.messages.get_last_user_message(&event.task_id).await {
        Ok(Some(user_msg)) => user_msg.agent_mode,
        _ => None,
    };

    // 4. Extract API message fields and build sdk_metadata
    let api_message = event.payload.get("message");

    let (
        assistant_message_id,
        assistant_message_model,
        assistant_stop_reason,
        assistant_stop_sequence,
    ) = if message_type == MessageType::Assistant {
        (
            api_message
                .and_then(|m| m.get("id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            api_message
                .and_then(|m| m.get("model"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            api_message
                .and_then(|m| m.get("stop_reason"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            api_message
                .and_then(|m| m.get("stop_sequence"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
        )
    } else {
        (None, None, None, None)
    };

    let sdk_metadata = if message_type == MessageType::Assistant {
        build_sdk_metadata(
            &event.payload,
            &["isApiErrorMessage", "apiError", "requestId"],
        )
    } else {
        build_sdk_metadata(
            &event.payload,
            &[
                "isMeta",
                "isCompactSummary",
                "isVisibleInTranscriptOnly",
                "planContent",
                "thinkingMetadata",
                "todos",
                "summarizeMetadata",
                "sourceToolAssistantUUID",
                "sourceToolUseID",
                "permissionMode",
            ],
        )
    };

    let tool_use_result = if message_type == MessageType::Assistant {
        None
    } else {
        event
            .payload
            .get("toolUseResult")
            .or_else(|| event.payload.get("tool_use_result"))
            .cloned()
    };

    // 5. Resolve turn number — events may carry one; otherwise inherit from
    //    the current task turn so the row groups with its user message.
    let turn_number = resolve_turn_number(state, &event.task_id, event.turn_number).await;

    // 6. Build TaskMessage
    let msg_row = TaskMessage {
        id: 0,
        task_id: event.task_id.clone(),
        uuid: uuid.to_string(),
        message_type,
        user_type: "system".to_string(),
        parent_uuid,
        is_sidechain: None,
        cwd: None,
        session_id,
        turn_number,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: event.timestamp.clone(),
        message_role,
        message_content: content.clone(),
        assistant_message_id,
        assistant_message_type: None,
        assistant_message_model,
        assistant_stop_reason,
        assistant_stop_sequence,
        assistant_usage_input_tokens: usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(serde_json::Value::as_i64)
            .map(|v| v as i32),
        assistant_usage_output_tokens: usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(serde_json::Value::as_i64)
            .map(|v| v as i32),
        assistant_usage_cache_creation_input_tokens: usage
            .and_then(|u| u.get("cache_creation_input_tokens"))
            .and_then(serde_json::Value::as_i64)
            .map(|v| v as i32),
        assistant_usage_cache_read_input_tokens: usage
            .and_then(|u| u.get("cache_read_input_tokens"))
            .and_then(serde_json::Value::as_i64)
            .map(|v| v as i32),
        tool_use_result,
        sdk_metadata,
        attachments: None,
        agent_session_id: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode,
        model: None,
        resume: true,
        sent_at: None,
        created_at: None,
        external_event_id: None,
        external_adapter: None,
        external_source_name: None,
        command_name: None,
        source_task_id: None,
        start_fresh: false,
        sent_by: None,
    };

    // 7. Build content blocks from the content array
    let blocks = build_content_blocks_from_payload(&content);
    let contexts: Vec<TaskMessageContext> = vec![];

    // 8. Persist message
    match state.messages.append(&msg_row, &contexts, &blocks).await {
        Ok(_) => {
            tracing::debug!(
                task_id = %event.task_id,
                uuid = %uuid,
                message_type = %message_type,
                "Agent message persisted"
            );
            crate::message::hooks::on_agent_message_persisted(
                state,
                &event.task_id,
                uuid,
                message_type,
                &content,
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(
                task_id = %event.task_id,
                uuid = %uuid,
                error = %e,
                "Failed to persist agent message"
            );
        }
    }

    // 9. Skill tagging (only for assistant messages)
    if message_type == MessageType::Assistant {
        let skill_names = extract_skill_names(&event.payload);
        if skill_names.is_empty() {
            return;
        }

        let tag_names: Vec<String> = skill_names
            .into_iter()
            .map(|s| format!("skill-{s}"))
            .collect();

        match state.tags.find_or_create_tags(&tag_names).await {
            Ok(tags) => {
                let tag_ids: Vec<String> = tags.into_iter().map(|t| t.tag_id).collect();
                if let Err(e) = state.tags.add_task_tags(&event.task_id, &tag_ids).await {
                    tracing::warn!(
                        task_id = %event.task_id,
                        error = %e,
                        "Failed to add skill tags"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %event.task_id,
                    error = %e,
                    "Failed to find/create skill tags"
                );
            }
        }
    }
}

/// Handle `task.metadata.updated` event.
pub async fn handle_task_metadata_updated(state: &AppState, event: &AgentEvent) {
    let name = event.payload.get("name").and_then(|v| v.as_str());
    let description = event.payload.get("description").and_then(|v| v.as_str());

    if name.is_none() && description.is_none() {
        return;
    }

    let patch = TaskPatch {
        name: name.map(std::string::ToString::to_string),
        description: description.map(std::string::ToString::to_string),
        ..Default::default()
    };

    if let Err(e) = state.tasks.update_task(&event.task_id, &patch).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task metadata");
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Build content block rows from a JSON content array.
///
/// Handles `text`, `tool_use`, and `tool_result` block types.
fn build_content_blocks_from_payload(content: &serde_json::Value) -> Vec<TaskMessageContentBlock> {
    let arr = match content.as_array() {
        Some(a) => a,
        None => return vec![],
    };

    arr.iter()
        .enumerate()
        .map(|(idx, block)| {
            let type_str = block.get("type").and_then(|v| v.as_str()).unwrap_or("text");

            let block_type = ContentBlockType::try_from(type_str).unwrap_or(ContentBlockType::Text);

            let (text_content, tool_use_id, tool_result_content, tool_result_is_error, show_in_ui) =
                match block_type {
                    ContentBlockType::Text => {
                        let text = block
                            .get("text")
                            .and_then(|v| v.as_str())
                            .map(std::string::ToString::to_string);
                        (text, None, None, None, true)
                    }
                    ContentBlockType::ToolUse => {
                        let id = block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(std::string::ToString::to_string);
                        (None, id, None, None, true)
                    }
                    ContentBlockType::ToolResult => {
                        let id = block
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .map(std::string::ToString::to_string);
                        let result_content = block.get("content").map(|v| {
                            if let Some(s) = v.as_str() {
                                s.to_string()
                            } else {
                                v.to_string()
                            }
                        });
                        let is_error = block.get("is_error").and_then(serde_json::Value::as_bool);
                        (None, id, result_content, is_error, false)
                    }
                    ContentBlockType::Image => (None, None, None, None, true),
                };

            TaskMessageContentBlock {
                id: 0,
                task_message_id: 0,
                sort_order: idx as i32,
                block_type,
                show_in_ui,
                text_content,
                tool_use_id,
                tool_result_content,
                tool_result_is_error,
                image_media_type: None,
                image_data: None,
                created_at: None,
            }
        })
        .collect()
}

/// Resolve which turn a result/error/stopped row belongs to.
///
/// Prefers the explicit `event.turn_number` when present (future code paths may
/// supply it). Falls back to the current max turn_number for the task, since a
/// terminal row always belongs to the most recent in-flight turn.
async fn resolve_turn_number(
    state: &AppState,
    task_id: &str,
    event_turn_number: Option<i32>,
) -> Option<i32> {
    if let Some(t) = event_turn_number {
        return Some(t);
    }
    match state.messages.current_turn_number(task_id).await {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!(task_id, error = %e, "Failed to read current turn number");
            None
        }
    }
}

/// Build a result message row for agent outcomes.
#[allow(clippy::too_many_arguments)]
fn build_result_message_row(
    task_id: &str,
    uuid: &str,
    timestamp: &str,
    content: serde_json::Value,
    message_type: MessageType,
    will_not_resume: bool,
    session_id: Option<String>,
    agent_mode: Option<AgentMode>,
    model: Option<String>,
    turn_number: Option<i32>,
) -> TaskMessage {
    TaskMessage {
        id: 0,
        task_id: task_id.to_string(),
        uuid: uuid.to_string(),
        message_type,
        user_type: "system".to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id: session_id.clone(),
        turn_number,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: timestamp.to_string(),
        message_role: MessageRole::Assistant,
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
        sdk_metadata: None,
        attachments: None,
        agent_session_id: session_id,
        will_not_resume: Some(will_not_resume),
        run_metrics: None,
        agent_mode,
        model,
        resume: true,
        sent_at: None,
        created_at: None,
        external_event_id: None,
        external_adapter: None,
        external_source_name: None,
        command_name: None,
        source_task_id: None,
        start_fresh: false,
        sent_by: None,
    }
}

/// Build sdk_metadata JSON object from selected keys in a payload.
/// Returns `None` if no keys matched or all were null.
fn build_sdk_metadata(payload: &serde_json::Value, keys: &[&str]) -> Option<serde_json::Value> {
    let mut meta = serde_json::Map::new();
    for key in keys {
        if let Some(v) = payload.get(*key) {
            if !v.is_null() {
                meta.insert((*key).to_string(), v.clone());
            }
        }
    }
    if meta.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(meta))
    }
}

/// Extract skill names from tool_use blocks in agent message payload.
fn extract_skill_names(payload: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();

    // Look for content blocks array in payload
    let content = payload
        .get("content")
        .or_else(|| payload.get("message_content"));

    let blocks = match content {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => return names,
    };

    for block in blocks {
        let block_type = block.get("type").and_then(|v| v.as_str());
        if block_type != Some("tool_use") {
            continue;
        }

        let tool_name = block.get("name").and_then(|v| v.as_str());
        if tool_name != Some("Skill") {
            continue;
        }

        // Extract skill name from tool input
        if let Some(input) = block.get("input") {
            if let Some(skill) = input.get("skill").and_then(|v| v.as_str()) {
                names.push(skill.to_string());
            }
        }
    }

    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── extract_skill_names ──────────────────────────────────────────

    #[test]
    fn extract_single_skill() {
        let payload = json!({
            "content": [{
                "type": "tool_use",
                "name": "Skill",
                "input": { "skill": "commit" }
            }]
        });
        assert_eq!(extract_skill_names(&payload), vec!["commit"]);
    }

    #[test]
    fn extract_multiple_skills() {
        let payload = json!({
            "content": [
                { "type": "tool_use", "name": "Skill", "input": { "skill": "commit" } },
                { "type": "tool_use", "name": "Skill", "input": { "skill": "review-pr" } }
            ]
        });
        assert_eq!(extract_skill_names(&payload), vec!["commit", "review-pr"]);
    }

    #[test]
    fn extract_ignores_non_skill_tools() {
        let payload = json!({
            "content": [
                { "type": "tool_use", "name": "Read", "input": { "path": "/tmp" } },
                { "type": "tool_use", "name": "Skill", "input": { "skill": "deploy" } }
            ]
        });
        assert_eq!(extract_skill_names(&payload), vec!["deploy"]);
    }

    #[test]
    fn extract_ignores_non_tool_use_blocks() {
        let payload = json!({
            "content": [
                { "type": "text", "text": "hello" },
                { "type": "tool_use", "name": "Skill", "input": { "skill": "test" } }
            ]
        });
        assert_eq!(extract_skill_names(&payload), vec!["test"]);
    }

    #[test]
    fn extract_empty_content() {
        let payload = json!({ "content": [] });
        assert!(extract_skill_names(&payload).is_empty());
    }

    #[test]
    fn extract_no_content_key() {
        let payload = json!({ "other": "data" });
        assert!(extract_skill_names(&payload).is_empty());
    }

    #[test]
    fn extract_from_message_content_key() {
        let payload = json!({
            "message_content": [{
                "type": "tool_use",
                "name": "Skill",
                "input": { "skill": "plan" }
            }]
        });
        assert_eq!(extract_skill_names(&payload), vec!["plan"]);
    }

    #[test]
    fn extract_skill_missing_input() {
        let payload = json!({
            "content": [{
                "type": "tool_use",
                "name": "Skill"
            }]
        });
        assert!(extract_skill_names(&payload).is_empty());
    }

    // ── message_uuid_for_event ───────────────────────────────────────

    #[test]
    fn message_uuid_deterministic_for_same_event_id() {
        // Same event_id → same uuid → ON CONFLICT (uuid) DO UPDATE upserts
        // instead of inserting a duplicate row on redelivery.
        let a = message_uuid_for_event("1700000000-0");
        let b = message_uuid_for_event("1700000000-0");
        assert_eq!(a, b);
    }

    #[test]
    fn message_uuid_distinct_for_distinct_event_ids() {
        // Different event_ids → different uuids → each event persists its own row.
        let a = message_uuid_for_event("1700000000-0");
        let b = message_uuid_for_event("1700000000-1");
        assert_ne!(a, b);
    }

    // ── collect_assistant_text ───────────────────────────────────────

    #[test]
    fn collect_assistant_text_concatenates_text_blocks() {
        let content = json!([
            { "type": "text", "text": "first" },
            { "type": "tool_use", "name": "Read", "input": {} },
            { "type": "text", "text": "second" }
        ]);
        assert_eq!(collect_assistant_text(&content), "first\n\nsecond");
    }

    #[test]
    fn collect_assistant_text_empty_when_no_text_blocks() {
        let content = json!([
            { "type": "tool_use", "name": "Read", "input": {} }
        ]);
        assert!(collect_assistant_text(&content).is_empty());
    }

    #[test]
    fn collect_assistant_text_empty_on_unexpected_shape() {
        let content = json!({ "not": "an array" });
        assert!(collect_assistant_text(&content).is_empty());
    }
}
