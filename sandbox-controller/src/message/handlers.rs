//! Message HTTP handlers.

use axum::extract::{Path, State};
use axum::Json;

use crate::db::message_dao;
use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::MessageError;
use super::transform;
use super::types::*;

/// GET /api/v1/tasks/{task_id}/messages
pub async fn get_messages(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<GetMessagesResponse>, MessageError> {
    let (messages, contexts, blocks) = message_dao::get_all(&state.db, &task_id).await?;
    let ui_messages = transform::transform_to_ui(&messages, &contexts, &blocks);
    let total_count = ui_messages.len();

    Ok(Json(GetMessagesResponse {
        messages: ui_messages,
        total_count,
    }))
}

/// GET /api/v1/tasks/{task_id}/messages/pending
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

/// DELETE /api/v1/tasks/{task_id}/messages/{message_id}
pub async fn delete_queued_message(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path((task_id, message_id)): Path<(String, String)>,
) -> Result<Json<DeleteMessageResponse>, MessageError> {
    let deleted = message_dao::delete_queued(&state.db, &task_id, &message_id).await?;
    if !deleted {
        return Err(MessageError::NotFound(message_id));
    }

    Ok(Json(DeleteMessageResponse {
        status: "deleted".to_string(),
    }))
}
