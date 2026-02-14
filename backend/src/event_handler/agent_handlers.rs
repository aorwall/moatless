//! Agent event handlers — process agent.completed, agent.stopped, agent.error events.

use moatless_events::{TaskSubType, TaskTurnSubType};

use crate::message::models::{TaskMessageContentBlock, TaskMessageContext, TaskMessage};
use crate::task::dao::TaskPatch;
use crate::message::models::{ContentBlockType, MessageRole, MessageType, TurnStatus};
use crate::state::AppState;
use crate::task::models::TaskStatus;

use super::models::{AgentEvent, SdkResultMessage};

/// Check if a result message already exists for the current turn (after the last user message).
/// Returns `true` if a duplicate should be skipped.
async fn has_result_for_current_turn(state: &AppState, task_id: &str) -> bool {
    let last_result = match state.messages.get_last_result(task_id).await {
        Ok(Some(r)) => r,
        _ => return false,
    };

    let (messages, _, _) = match state.messages.get_all(task_id).await {
        Ok(data) => data,
        Err(_) => return false,
    };

    let last_user_ts = messages
        .iter()
        .rev()
        .find(|m| m.message_type == MessageType::User)
        .map(|m| m.timestamp.as_str());

    match last_user_ts {
        Some(user_ts) => last_result.timestamp.as_str() >= user_ts,
        None => true, // No user messages at all, result already exists
    }
}

/// Handle `agent.completed` event.
///
/// Parses the SDK result, determines turn status, appends result message,
/// updates task, and emits events.
pub async fn handle_agent_completed(state: &AppState, event: &AgentEvent) {
    // Idempotency: skip if a result already exists for the current turn
    if has_result_for_current_turn(state, &event.task_id).await {
        tracing::debug!(task_id = %event.task_id, "Skipping agent.completed — result already exists for current turn");
        return;
    }
    let sdk_result: Option<SdkResultMessage> =
        match serde_json::from_value(event.payload.clone()) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!(
                    task_id = %event.task_id,
                    error = %e,
                    "Failed to parse agent.completed payload — treating as error"
                );
                None
            }
        };

    let (turn_status, will_not_resume, task_status) = match &sdk_result {
        Some(r) if r.is_error.unwrap_or(false) => (TurnStatus::Error, true, TaskStatus::Error),
        Some(_) => (TurnStatus::Completed, false, TaskStatus::Open),
        None => (TurnStatus::Error, true, TaskStatus::Error),
    };

    // Build result text
    let result_text = build_result_text(&sdk_result);

    // Extract session_id from SDK result
    let session_id = sdk_result.as_ref().and_then(|r| r.session_id.clone());

    // Fallback: set real Claude Code session_id on user messages
    // (in case session_init event was missed or arrived before messages were fetched)
    if let Some(ref sid) = session_id {
        if let Err(e) = state.messages.set_session_id_for_task(&event.task_id, sid).await {
            tracing::warn!(task_id = %event.task_id, error = %e, "Failed to set session_id on user messages (fallback)");
        }
    }

    // Extract model from SDK result payload (first key in model_usage map)
    let model = event
        .payload
        .get("model_usage")
        .or_else(|| event.payload.get("modelUsage"))
        .and_then(|mu| mu.as_object())
        .and_then(|obj| obj.keys().next())
        .map(|s| s.to_string());

    // Inherit agent_mode from the last user message for this task
    let agent_mode = match state.messages.get_last_user_message(&event.task_id).await {
        Ok(Some(user_msg)) => user_msg.agent_mode,
        _ => None,
    };

    // Append result message
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    let content = serde_json::json!([{
        "type": "text",
        "text": result_text
    }]);

    let msg_row = build_result_message_row(&event.task_id, &msg_uuid, &now, content, turn_status, will_not_resume, session_id, agent_mode, model);
    let contexts: Vec<TaskMessageContext> = vec![];
    let blocks = vec![TaskMessageContentBlock {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: ContentBlockType::Text,
        show_in_ui: true,
        text_content: Some(result_text),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    }];

    if let Err(e) = state.messages.append(&msg_row, &contexts, &blocks).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to append result message");
    }

    // Update task
    let mut patch = TaskPatch {
        status: Some(task_status),
        last_message_at: Some(now.clone()),
        ..Default::default()
    };

    if let Some(ref r) = sdk_result {
        if let Some(cost) = r.total_cost_usd {
            patch.total_cost_usd_increment = Some(cost);
        }
    }

    if let Err(e) = state.tasks.update_task(&event.task_id, &patch).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task after completion");
    }

    // Emit events
    if let Some(ref events) = state.events {
        let turn_sub = match turn_status {
            TurnStatus::Error => TaskTurnSubType::Error,
            TurnStatus::Completed => TaskTurnSubType::Completed,
            TurnStatus::Stopped => TaskTurnSubType::Stopped,
        };
        events
            .publish_task_turn_event(
                &event.task_id,
                turn_sub,
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

/// Handle `agent.session_init` event — Claude Code session started.
///
/// Sets the real Claude Code session_id on user messages that have been
/// fetched by the sandbox (sent_at IS NOT NULL) but don't yet have a session_id.
pub async fn handle_agent_session_init(state: &AppState, event: &AgentEvent) {
    let session_id = match event.payload.get("session_id").and_then(|v| v.as_str()) {
        Some(sid) => sid,
        None => {
            tracing::warn!(task_id = %event.task_id, "agent.session_init missing session_id");
            return;
        }
    };

    match state.messages.set_session_id_for_task(&event.task_id, session_id).await {
        Ok(count) => {
            tracing::info!(
                task_id = %event.task_id,
                session_id = %session_id,
                count,
                "Set Claude Code session_id on user messages"
            );
        }
        Err(e) => {
            tracing::warn!(
                task_id = %event.task_id,
                error = %e,
                "Failed to set session_id on user messages"
            );
        }
    }
}

/// Handle `agent.stopped` event — user requested stop.
pub async fn handle_agent_stopped(state: &AppState, event: &AgentEvent) {
    // Idempotency: skip if a result already exists for the current turn
    if has_result_for_current_turn(state, &event.task_id).await {
        tracing::debug!(task_id = %event.task_id, "Skipping agent.stopped — result already exists for current turn");
        return;
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    let content = serde_json::json!([{
        "type": "text",
        "text": "Stopped by user request"
    }]);

    // Inherit agent_mode from the last user message
    let agent_mode = match state.messages.get_last_user_message(&event.task_id).await {
        Ok(Some(user_msg)) => user_msg.agent_mode,
        _ => None,
    };

    let msg_row = build_result_message_row(&event.task_id, &msg_uuid, &now, content, TurnStatus::Stopped, true, None, agent_mode, None);
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

    if let Err(e) = state.tasks.update_task(
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
}

/// Handle `agent.error` event.
pub async fn handle_agent_error(state: &AppState, event: &AgentEvent) {
    // Guard: only create error result if task has messages and last isn't already a result
    match state.messages.get_last_message_type(&event.task_id).await {
        Ok(None) => {
            tracing::debug!(task_id = %event.task_id, "Skipping agent.error — no messages exist for task");
            return;
        }
        Ok(Some(MessageType::Result)) => {
            tracing::debug!(task_id = %event.task_id, "Skipping agent.error — last message is already a result");
            return;
        }
        Ok(Some(_)) => {} // Has messages, last is not result — proceed
        Err(e) => {
            tracing::warn!(task_id = %event.task_id, error = %e, "Failed to check message state for agent.error");
            return; // Fail safe: don't create orphan results
        }
    }

    let error_msg = event
        .payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown agent error");

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    let content = serde_json::json!([{
        "type": "text",
        "text": error_msg
    }]);

    // Inherit agent_mode from the last user message
    let agent_mode = match state.messages.get_last_user_message(&event.task_id).await {
        Ok(Some(user_msg)) => user_msg.agent_mode,
        _ => None,
    };

    let msg_row = build_result_message_row(&event.task_id, &msg_uuid, &now, content, TurnStatus::Error, true, None, agent_mode, None);
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

    if let Err(e) = state.tasks.update_task(
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
        events
            .publish_task_turn_event(
                &event.task_id,
                TaskTurnSubType::Error,
                event.agent_type.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;
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
        .map(|s| s.to_string());

    let parent_uuid = event
        .payload
        .get("parent_tool_use_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

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

    let usage = event
        .payload
        .get("message")
        .and_then(|m| m.get("usage"));

    // 3. Inherit agent_mode from the last user message (same pattern as result messages)
    let agent_mode = match state.messages.get_last_user_message(&event.task_id).await {
        Ok(Some(user_msg)) => user_msg.agent_mode,
        _ => None,
    };

    // 4. Build TaskMessage
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
        turn_number: event.turn_number,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: event.timestamp.clone(),
        message_role,
        message_content: content.clone(),
        assistant_message_id: None,
        assistant_message_type: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        assistant_usage_output_tokens: usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        assistant_usage_cache_creation_input_tokens: usage
            .and_then(|u| u.get("cache_creation_input_tokens"))
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        assistant_usage_cache_read_input_tokens: usage
            .and_then(|u| u.get("cache_read_input_tokens"))
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        tool_use_result: None,
        attachments: None,
        agent_session_id: None,
        turn_status: None,
        turn_result: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode,
        model: None,
        resume_session_id: None,
        sent_at: None,
        created_at: None,
    };

    // 5. Build content blocks from the content array
    let blocks = build_content_blocks_from_payload(&content);
    let contexts: Vec<TaskMessageContext> = vec![];

    // 6. Persist message
    match state.messages.append(&msg_row, &contexts, &blocks).await {
        Ok(_) => {
            tracing::info!(
                task_id = %event.task_id,
                uuid = %uuid,
                message_type = %message_type,
                "Agent message persisted"
            );
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

    // 7. Skill tagging (only for assistant messages)
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
                if let Err(e) =
                    state.tags.add_task_tags(&event.task_id, &tag_ids).await
                {
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
        name: name.map(|s| s.to_string()),
        description: description.map(|s| s.to_string()),
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
            let type_str = block
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("text");

            let block_type = ContentBlockType::try_from(type_str)
                .unwrap_or(ContentBlockType::Text);

            let (text_content, tool_use_id, tool_result_content, tool_result_is_error, show_in_ui) =
                match block_type {
                    ContentBlockType::Text => {
                        let text = block.get("text").and_then(|v| v.as_str()).map(|s| s.to_string());
                        (text, None, None, None, true)
                    }
                    ContentBlockType::ToolUse => {
                        let id = block.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                        (None, id, None, None, true)
                    }
                    ContentBlockType::ToolResult => {
                        let id = block
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let result_content = block
                            .get("content")
                            .map(|v| {
                                if let Some(s) = v.as_str() {
                                    s.to_string()
                                } else {
                                    v.to_string()
                                }
                            });
                        let is_error = block.get("is_error").and_then(|v| v.as_bool());
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

/// Build result text from SDK result message.
fn build_result_text(sdk_result: &Option<SdkResultMessage>) -> String {
    match sdk_result {
        Some(r) => {
            let default_subtype = if r.is_error.unwrap_or(false) { "error" } else { "completed" };
            let subtype = r.subtype.as_deref().unwrap_or(default_subtype);
            if r.is_error.unwrap_or(false) {
                let error_detail = r
                    .result
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                format!("Agent {subtype} with error: {error_detail}")
            } else {
                let result_detail = r
                    .result
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if result_detail.is_empty() {
                    format!("Agent {subtype}")
                } else {
                    format!("Agent {subtype}: {result_detail}")
                }
            }
        }
        None => "Agent completed (unreadable result)".to_string(),
    }
}

/// Build a result message row for agent outcomes.
#[allow(clippy::too_many_arguments)]
fn build_result_message_row(
    task_id: &str,
    uuid: &str,
    timestamp: &str,
    content: serde_json::Value,
    turn_status: TurnStatus,
    will_not_resume: bool,
    session_id: Option<String>,
    agent_mode: Option<String>,
    model: Option<String>,
) -> TaskMessage {
    TaskMessage {
        id: 0,
        task_id: task_id.to_string(),
        uuid: uuid.to_string(),
        message_type: MessageType::Result,
        user_type: "system".to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id,
        turn_number: None,
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
        attachments: None,
        agent_session_id: None,
        turn_status: Some(turn_status),
        turn_result: None,
        will_not_resume: Some(will_not_resume),
        run_metrics: None,
        agent_mode,
        model,
        resume_session_id: None,
        sent_at: None,
        created_at: None,
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

    // ── build_result_text ────────────────────────────────────────────

    #[test]
    fn build_result_text_none() {
        assert_eq!(
            build_result_text(&None),
            "Agent completed (unreadable result)"
        );
    }

    #[test]
    fn build_result_text_success_with_detail() {
        let r = SdkResultMessage {
            session_id: None,
            is_error: Some(false),
            subtype: Some("completed".to_string()),
            result: Some(json!("All tasks done")),
            total_cost_usd: None,
            usage: None,
            num_turns: None,
            duration_ms: None,
        };
        assert_eq!(build_result_text(&Some(r)), "Agent completed: All tasks done");
    }

    #[test]
    fn build_result_text_success_empty_result() {
        let r = SdkResultMessage {
            session_id: None,
            is_error: Some(false),
            subtype: Some("finished".to_string()),
            result: None,
            total_cost_usd: None,
            usage: None,
            num_turns: None,
            duration_ms: None,
        };
        assert_eq!(build_result_text(&Some(r)), "Agent finished");
    }

    #[test]
    fn build_result_text_error() {
        let r = SdkResultMessage {
            session_id: None,
            is_error: Some(true),
            subtype: Some("crashed".to_string()),
            result: Some(json!("OOM")),
            total_cost_usd: None,
            usage: None,
            num_turns: None,
            duration_ms: None,
        };
        assert_eq!(build_result_text(&Some(r)), "Agent crashed with error: OOM");
    }

    #[test]
    fn build_result_text_missing_subtype() {
        let r = SdkResultMessage {
            session_id: None,
            is_error: None,
            subtype: None,
            result: Some(json!("done")),
            total_cost_usd: None,
            usage: None,
            num_turns: None,
            duration_ms: None,
        };
        assert_eq!(build_result_text(&Some(r)), "Agent completed: done");
    }
}
