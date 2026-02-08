//! Agent HTTP handlers.

use axum::extract::{Path, State};
use axum::Json;

use crate::db::{command_dao, task_dao};
use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::AgentError;
use super::orchestration;
use super::types::*;

/// POST /api/v1/tasks/{task_id}/messages
pub async fn send_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, AgentError> {
    // Verify task exists and user has access
    let _task = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| AgentError::NotFound(task_id.clone()))?;

    // Handle tool response (AskUserQuestion / ExitPlanMode)
    if let (Some(_tool_use_id), Some(_tool_name)) = (&req.tool_use_id, &req.tool_name) {
        // Save tool result message and notify
        orchestration::send_tool_response_flow(
            &state,
            &task_id,
            req.tool_use_id.as_deref().unwrap_or_default(),
            req.tool_name.as_deref().unwrap_or_default(),
            req.tool_response.as_ref(),
        )
        .await?;
    } else {
        // Regular message
        orchestration::send_message_flow(
            &state,
            &task_id,
            &req.message,
            req.model.as_deref(),
            req.agent_mode.as_deref(),
            &req.file_ids,
            &req.contexts,
        )
        .await?;
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(Json(SendMessageResponse {
        status: "sent".to_string(),
        timestamp: now,
    }))
}

/// POST /api/v1/tasks/{task_id}/commands
pub async fn send_command(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SendCommandRequest>,
) -> Result<Json<SendMessageResponse>, AgentError> {
    let _task = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| AgentError::NotFound(task_id.clone()))?;

    let command = command_dao::find_by_name(&state.db, &req.command_name)
        .await?
        .ok_or_else(|| AgentError::CommandNotFound(req.command_name.clone()))?;

    orchestration::send_message_flow(
        &state,
        &task_id,
        &command.prompt,
        req.model.as_deref(),
        command.agent_mode.as_deref(),
        &req.file_ids,
        &[],
    )
    .await?;

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(Json(SendMessageResponse {
        status: "sent".to_string(),
        timestamp: now,
    }))
}

/// POST /api/v1/tasks/{task_id}/stop
pub async fn stop_agent(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, AgentError> {
    let _task = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| AgentError::NotFound(task_id.clone()))?;

    // Stop the sandbox agent process
    if let Err(e) = state.sandbox.stop(&task_id).await {
        tracing::warn!(task_id = %task_id, error = %e, "Failed to stop sandbox agent");
    }

    // Update task status
    task_dao::update_task(
        &state.db,
        &task_id,
        &crate::db::task_dao::TaskPatch {
            status: Some("open".to_string()),
            ..Default::default()
        },
    )
    .await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}
