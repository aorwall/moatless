//! Agent event handlers — process agent.completed, agent.stopped, agent.error events.

use crate::db::{message_dao, tag_dao, task_dao};
use crate::db::models::{TaskMessageContentBlockRow, TaskMessageContextRow, TaskMessageRow};
use crate::db::task_dao::TaskPatch;
use crate::state::AppState;

use super::types::{AgentEvent, SdkResultMessage};

/// Handle `agent.completed` event.
///
/// Parses the SDK result, determines turn status, appends result message,
/// updates task, and emits events.
pub async fn handle_agent_completed(state: &AppState, event: &AgentEvent) {
    let sdk_result: Option<SdkResultMessage> =
        serde_json::from_value(event.payload.clone()).ok();

    let (turn_status, will_not_resume, task_status) = match &sdk_result {
        Some(r) if r.is_error.unwrap_or(false) => ("error", true, "error"),
        _ => ("completed", false, "open"),
    };

    // Build result text
    let result_text = build_result_text(&sdk_result);

    // Append result message
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    let content = serde_json::json!([{
        "type": "text",
        "text": result_text
    }]);

    let msg_row = build_result_message_row(&event.task_id, &msg_uuid, &now, content, turn_status, will_not_resume);
    let contexts: Vec<TaskMessageContextRow> = vec![];
    let blocks = vec![TaskMessageContentBlockRow {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: "text".to_string(),
        show_in_ui: true,
        text_content: Some(result_text),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    }];

    if let Err(e) = message_dao::append(&state.db, &msg_row, &contexts, &blocks).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to append result message");
    }

    // Update task
    let mut patch = TaskPatch {
        status: Some(task_status.to_string()),
        last_message_at: Some(now.clone()),
        ..Default::default()
    };

    if let Some(ref r) = sdk_result {
        if let Some(cost) = r.total_cost_usd {
            patch.total_cost_usd_increment = Some(cost);
        }
    }

    if let Err(e) = task_dao::update_task(&state.db, &event.task_id, &patch).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task after completion");
    }

    // Emit events
    if let Some(ref events) = state.events {
        events
            .publish_task_turn_event(
                &event.task_id,
                turn_status,
                event.agent_type.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;

        events
            .publish_task_event(
                &event.task_id,
                task_status,
                event.agent_type.as_deref(),
                serde_json::json!({}),
            )
            .await;
    }
}

/// Handle `agent.stopped` event — user requested stop.
pub async fn handle_agent_stopped(state: &AppState, event: &AgentEvent) {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    let content = serde_json::json!([{
        "type": "text",
        "text": "Stopped by user request"
    }]);

    let msg_row = build_result_message_row(&event.task_id, &msg_uuid, &now, content, "stopped", true);
    let contexts: Vec<TaskMessageContextRow> = vec![];
    let blocks = vec![TaskMessageContentBlockRow {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: "text".to_string(),
        show_in_ui: true,
        text_content: Some("Stopped by user request".to_string()),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    }];

    if let Err(e) = message_dao::append(&state.db, &msg_row, &contexts, &blocks).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to append stopped message");
    }

    if let Err(e) = task_dao::update_task(
        &state.db,
        &event.task_id,
        &TaskPatch {
            status: Some("open".to_string()),
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
                "stopped",
                event.agent_type.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;
    }
}

/// Handle `agent.error` event.
pub async fn handle_agent_error(state: &AppState, event: &AgentEvent) {
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

    let msg_row = build_result_message_row(&event.task_id, &msg_uuid, &now, content, "error", true);
    let contexts: Vec<TaskMessageContextRow> = vec![];
    let blocks = vec![TaskMessageContentBlockRow {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: "text".to_string(),
        show_in_ui: true,
        text_content: Some(error_msg.to_string()),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    }];

    if let Err(e) = message_dao::append(&state.db, &msg_row, &contexts, &blocks).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to append error message");
    }

    if let Err(e) = task_dao::update_task(
        &state.db,
        &event.task_id,
        &TaskPatch {
            status: Some("error".to_string()),
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
                "error",
                event.agent_type.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;
    }
}

/// Handle `agent.message` event — auto-tag tasks based on tool usage.
///
/// Looks for `tool_use` blocks where tool name is "Skill" and creates
/// tags like `skill-{name}`. Fire-and-forget (logs on error, never fails).
pub async fn handle_agent_message(state: &AppState, event: &AgentEvent) {
    // Extract skill names from tool_use blocks in the payload
    let skill_names = extract_skill_names(&event.payload);
    if skill_names.is_empty() {
        return;
    }

    // Create tags from skill names
    let tag_names: Vec<String> = skill_names
        .into_iter()
        .map(|s| format!("skill-{s}"))
        .collect();

    match tag_dao::find_or_create_tags(&state.db, &tag_names).await {
        Ok(tags) => {
            let tag_ids: Vec<String> = tags.into_iter().map(|t| t.tag_id).collect();
            if let Err(e) = tag_dao::add_task_tags(&state.db, &event.task_id, &tag_ids).await {
                tracing::warn!(task_id = %event.task_id, error = %e, "Failed to add skill tags");
            }
        }
        Err(e) => {
            tracing::warn!(task_id = %event.task_id, error = %e, "Failed to find/create skill tags");
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

    if let Err(e) = task_dao::update_task(&state.db, &event.task_id, &patch).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task metadata");
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Build result text from SDK result message.
fn build_result_text(sdk_result: &Option<SdkResultMessage>) -> String {
    match sdk_result {
        Some(r) => {
            let subtype = r.subtype.as_deref().unwrap_or("unknown");
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
        None => "Agent completed (no result data)".to_string(),
    }
}

/// Build a result message row for agent outcomes.
fn build_result_message_row(
    task_id: &str,
    uuid: &str,
    timestamp: &str,
    content: serde_json::Value,
    turn_status: &str,
    will_not_resume: bool,
) -> TaskMessageRow {
    TaskMessageRow {
        id: 0,
        task_id: task_id.to_string(),
        uuid: uuid.to_string(),
        message_type: "result".to_string(),
        user_type: "system".to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id: None,
        turn_number: None,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: timestamp.to_string(),
        message_role: "assistant".to_string(),
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
        turn_status: Some(turn_status.to_string()),
        turn_result: None,
        will_not_resume: Some(will_not_resume),
        run_metrics: None,
        agent_mode: None,
        model: None,
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
            "Agent completed (no result data)"
        );
    }

    #[test]
    fn build_result_text_success_with_detail() {
        let r = SdkResultMessage {
            is_error: Some(false),
            subtype: Some("completed".to_string()),
            result: Some(json!("All tasks done")),
            total_cost_usd: None,
            usage: None,
        };
        assert_eq!(build_result_text(&Some(r)), "Agent completed: All tasks done");
    }

    #[test]
    fn build_result_text_success_empty_result() {
        let r = SdkResultMessage {
            is_error: Some(false),
            subtype: Some("finished".to_string()),
            result: None,
            total_cost_usd: None,
            usage: None,
        };
        assert_eq!(build_result_text(&Some(r)), "Agent finished");
    }

    #[test]
    fn build_result_text_error() {
        let r = SdkResultMessage {
            is_error: Some(true),
            subtype: Some("crashed".to_string()),
            result: Some(json!("OOM")),
            total_cost_usd: None,
            usage: None,
        };
        assert_eq!(build_result_text(&Some(r)), "Agent crashed with error: OOM");
    }

    #[test]
    fn build_result_text_missing_subtype() {
        let r = SdkResultMessage {
            is_error: None,
            subtype: None,
            result: Some(json!("done")),
            total_cost_usd: None,
            usage: None,
        };
        assert_eq!(build_result_text(&Some(r)), "Agent unknown: done");
    }
}
