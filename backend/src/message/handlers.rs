//! Message HTTP handlers.

use axum::extract::{Path, State};
use axum::Json;

use crate::command::dao as command_dao;
use super::dao as message_dao;
use crate::task::dao as task_dao;
use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::MessageError;
use super::send;
use super::transform;
use super::schemas::*;

/// Send a message to the agent for a task.
///
/// If `command_name` is set, the command's prompt is used instead of `message`.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/messages",
    tag = "messages",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = SendMessageRequest,
    responses(
        (status = 200, description = "Message sent", body = SendMessageResponse),
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn send_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, MessageError> {
    // Verify task exists and user has access
    let _task = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| MessageError::NotFound(task_id.clone()))?;

    // Handle tool response (AskUserQuestion / ExitPlanMode)
    if let (Some(_tool_use_id), Some(_tool_name)) = (&req.tool_use_id, &req.tool_name) {
        send::send_tool_response_flow(
            &state,
            &task_id,
            req.tool_use_id.as_deref().unwrap_or_default(),
            req.tool_name.as_deref().unwrap_or_default(),
            req.tool_response.as_ref(),
        )
        .await?;
    } else if let Some(ref command_name) = req.command_name {
        // Named command — look up prompt and agent_mode from DB
        let command = command_dao::find_by_name(&state.db, command_name)
            .await?
            .ok_or_else(|| MessageError::CommandNotFound(command_name.clone()))?;

        send::send_message_flow(
            &state,
            &task_id,
            &command.prompt,
            req.model.as_deref(),
            command.agent_mode.as_deref(),
            &req.file_ids,
            &[],
        )
        .await?;
    } else {
        // Regular message
        send::send_message_flow(
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

    // Notify the sandbox pod that a new message is available (fire-and-forget).
    // This is only done here (not in send flows) because task creation callers
    // skip notification — the sandbox reads pending messages on startup.
    crate::sandbox::notify::notify_sandbox_for_task(&state, &task_id).await;

    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Message sent to agent");

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(Json(SendMessageResponse {
        status: "sent".to_string(),
        timestamp: now,
    }))
}

/// Get all messages for a task.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/messages",
    tag = "messages",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "List of messages", body = GetMessagesResponse),
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn get_messages(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<GetMessagesResponse>, MessageError> {
    let (messages, contexts, blocks) = message_dao::get_all(&state.db, &task_id).await?;
    let ui_messages: Vec<UiMessage> = transform::transform_to_ui(&messages, &contexts, &blocks)
        .into_iter()
        .filter(|m| {
            !m.content.is_empty()
                || !m.tool_results.is_empty()
                || m.result.is_some()
                || m.has_pending_user_input == Some(true)
        })
        .collect();
    let total_count = ui_messages.len();

    Ok(Json(GetMessagesResponse {
        messages: ui_messages,
        total_count,
    }))
}

/// Get pending messages for a task.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/messages/pending",
    tag = "messages",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Pending messages", body = GetPendingMessagesResponse),
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn get_pending_messages(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<GetPendingMessagesResponse>, MessageError> {
    let pending = message_dao::get_pending_user(&state.db, &task_id).await?;
    let last_result = message_dao::get_last_result(&state.db, &task_id).await?;

    let mut messages: Vec<serde_json::Value> = pending
        .iter()
        .map(|m| serde_json::json!({
            "uuid": m.uuid,
            "messageType": m.message_type,
            "timestamp": m.timestamp,
            "messageContent": m.message_content,
            "agentMode": m.agent_mode,
            "model": m.model,
            "resumeSessionId": m.resume_session_id,
        }))
        .collect();

    if let Some(result) = last_result {
        messages.push(serde_json::json!({
            "uuid": result.uuid,
            "messageType": result.message_type,
            "timestamp": result.timestamp,
            "turnStatus": result.turn_status,
            "turnResult": result.turn_result,
            "willNotResume": result.will_not_resume,
        }));
    }

    let total_count = messages.len();
    Ok(Json(GetPendingMessagesResponse {
        messages,
        total_count,
    }))
}

/// Delete a queued message.
#[utoipa::path(
    delete,
    path = "/api/v1/tasks/{task_id}/messages/{message_id}",
    tag = "messages",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("message_id" = String, Path, description = "Message identifier")
    ),
    responses(
        (status = 200, description = "Message deleted", body = DeleteMessageResponse),
        (status = 404, description = "Message not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn delete_queued_message(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path((task_id, message_id)): Path<(String, String)>,
) -> Result<Json<DeleteMessageResponse>, MessageError> {
    let deleted = message_dao::delete_queued(&state.db, &task_id, &message_id).await?;
    if !deleted {
        return Err(MessageError::NotFound(message_id));
    }

    tracing::info!(task_id = %task_id, message_id = %message_id, "Queued message deleted");

    Ok(Json(DeleteMessageResponse {
        status: "deleted".to_string(),
    }))
}
