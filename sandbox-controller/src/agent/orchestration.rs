//! Agent orchestration — free async functions for message/task flows.

use std::sync::Arc;

use crate::db::{message_dao, task_dao};
use crate::db::models::{TaskMessageContentBlockRow, TaskMessageContextRow, TaskMessageRow};
use crate::db::task_dao::TaskPatch;
use crate::sandbox::k8s::naming::generate_service_name;
use crate::state::AppState;
use crate::task::types::MessageContextInput;

use super::error::AgentError;
use super::notify;
use super::request_builder;

/// The main message flow — called by handlers and by task creation.
pub async fn send_message_flow(
    state: &AppState,
    task_id: &str,
    message: &str,
    model: Option<&str>,
    agent_mode: Option<&str>,
    file_ids: &[String],
    contexts: &[MessageContextInput],
) -> Result<(), AgentError> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    // Build enriched content blocks (includes figma, preview, element contexts + file attachments)
    let enriched_blocks =
        request_builder::build_content_blocks(&state.db, &state.config, message, contexts, file_ids).await;
    let content = serde_json::to_value(&enriched_blocks).unwrap_or_else(|_| {
        serde_json::json!([{ "type": "text", "text": message }])
    });

    let msg_row = TaskMessageRow {
        id: 0, // will be assigned by DB
        task_id: task_id.to_string(),
        uuid: msg_uuid,
        message_type: "user".to_string(),
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
        message_role: "user".to_string(),
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
    let ui_block = TaskMessageContentBlockRow {
        id: 0,
        task_message_id: 0, // will be set by DAO
        sort_order: 0,
        block_type: "text".to_string(),
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
    let contexts: Vec<TaskMessageContextRow> = vec![];
    message_dao::append(&state.db, &msg_row, &contexts, &[ui_block])
        .await
        .map_err(|e| AgentError::Internal(format!("Failed to persist message: {e}")))?;

    // Update task status to running
    task_dao::update_task(
        &state.db,
        task_id,
        &TaskPatch {
            status: Some("running".to_string()),
            last_message_at: Some(now),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| AgentError::Internal(format!("Failed to update task status: {e}")))?;

    // Notify sandbox pod (fire-and-forget)
    let svc_name = generate_service_name(task_id);
    let namespace = state.config.sandbox_namespace.clone();
    notify::notify_sandbox(&namespace, &svc_name).await;

    // Ensure workspace is provisioned in background
    let sandbox = Arc::clone(&state.sandbox);
    let tid = task_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = sandbox.start(&tid).await {
            tracing::warn!(task_id = %tid, error = %e, "Background workspace start failed");
        }
    });

    Ok(())
}

/// Send a tool response (AskUserQuestion / ExitPlanMode).
pub async fn send_tool_response_flow(
    state: &AppState,
    task_id: &str,
    tool_use_id: &str,
    _tool_name: &str,
    tool_response: Option<&serde_json::Value>,
) -> Result<(), AgentError> {
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

    let msg_row = TaskMessageRow {
        id: 0,
        task_id: task_id.to_string(),
        uuid: msg_uuid,
        message_type: "user".to_string(),
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
        message_role: "user".to_string(),
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

    let contexts: Vec<TaskMessageContextRow> = vec![];
    let blocks: Vec<TaskMessageContentBlockRow> = vec![];
    message_dao::append(&state.db, &msg_row, &contexts, &blocks)
        .await
        .map_err(|e| AgentError::Internal(format!("Failed to persist tool response: {e}")))?;

    // Notify sandbox
    let svc_name = generate_service_name(task_id);
    notify::notify_sandbox(&state.config.sandbox_namespace, &svc_name).await;

    Ok(())
}

/// Start task flow — wait for workspace ready, then set status.
pub async fn start_task_flow(state: &AppState, task_id: &str) -> Result<(), AgentError> {
    // Poll workspace status with timeout
    let timeout = tokio::time::Duration::from_secs(30);
    let start = tokio::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            return Err(AgentError::Internal("Workspace not ready after 30s".to_string()));
        }

        let status = state.sandbox.status(task_id).await;
        match status {
            Ok(s) => {
                let env_status = format!("{:?}", s.status);
                if env_status.contains("Ready") || env_status.contains("Running") {
                    break;
                }
            }
            Err(_) => {}
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Update task status
    task_dao::update_task(
        &state.db,
        task_id,
        &TaskPatch {
            status: Some("running".to_string()),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| AgentError::Internal(format!("Failed to update task: {e}")))?;

    // Emit event
    if let Some(ref events) = state.events {
        events
            .publish_task_turn_event(
                task_id,
                "active",
                None,
                None,
                serde_json::json!({}),
            )
            .await;
    }

    // Notify sandbox
    let svc_name = generate_service_name(task_id);
    notify::notify_sandbox(&state.config.sandbox_namespace, &svc_name).await;

    Ok(())
}
