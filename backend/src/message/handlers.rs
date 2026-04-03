//! Message HTTP handlers.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;

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
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn send_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, MessageError> {
    // Verify task exists and user has access
    let _task = state.tasks.find_by_id(&task_id, &auth.scope_context())
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
        let command = state.commands.find_by_name(command_name)
            .await?
            .ok_or_else(|| MessageError::CommandNotFound(command_name.clone()))?;

        send::send_message_flow(
            &state,
            &task_id,
            &command.prompt,
            req.model.as_deref(),
            command.agent_mode.as_ref(),
            &req.file_ids,
            &[],
            req.resume.unwrap_or(true),
            None,
            None,
            None,
        )
        .await?;
    } else {
        // Regular message
        send::send_message_flow(
            &state,
            &task_id,
            &req.message,
            req.model.as_deref(),
            req.agent_mode.as_ref(),
            &req.file_ids,
            &req.contexts,
            req.resume.unwrap_or(true),
            None,
            None,
            None,
        )
        .await?;
    }

    // Ensure the sandbox is running (auto-provisions if needed).
    // Only notify if already running — a starting sandbox fetches pending messages on startup.
    match crate::sandbox::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
        &task_id,
    )
    .await
    {
        Ok(request) => {
            match state.sandbox.provision(&task_id, &request).await {
                Ok(resp) if resp.status == crate::sandbox::models::EnvironmentStatus::Ready => {
                    // Sandbox was already running — notify it about the new message.
                    crate::sandbox::notify::notify_sandbox_for_task(&state, &task_id).await;
                }
                Ok(_) => {
                    // Sandbox is starting — it will fetch pending messages on startup.
                    tracing::info!(task_id = %task_id, "Sandbox starting, skipping notification");
                }
                Err(e) => {
                    tracing::warn!(task_id = %task_id, error = %e, "Failed to provision sandbox for message delivery");
                    send::persist_error_result(
                        &state,
                        &task_id,
                        &format!("Failed to provision sandbox: {e}"),
                    )
                    .await;
                }
            }
        }
        Err(e) => {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to build provision request for message delivery");
            send::persist_error_result(
                &state,
                &task_id,
                &format!("Failed to build provision request: {e}"),
            )
            .await;
        }
    }

    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Message sent to agent");

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(Json(SendMessageResponse {
        status: "sent".to_string(),
        timestamp: now,
    }))
}

/// Query parameters for GET /messages.
#[derive(Debug, Deserialize)]
pub struct GetMessagesQuery {
    /// When true, strip heavy fields (tool_input, typed_result, sdk_metadata)
    /// and truncate tool result content. Useful for CLI/summary views.
    #[serde(default)]
    pub summary: bool,
}

/// Get all messages for a task.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/messages",
    tag = "messages",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("summary" = Option<bool>, Query, description = "Strip heavy fields for summary view")
    ),
    responses(
        (status = 200, description = "List of messages", body = GetMessagesResponse),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_messages(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<GetMessagesQuery>,
) -> Result<Json<GetMessagesResponse>, MessageError> {
    let (messages, contexts, blocks) = state.messages.get_all(&task_id).await?;
    let ui_messages: Vec<UiMessage> = transform::transform_to_ui(&messages, &contexts, &blocks)
        .into_iter()
        .filter(|m| {
            !m.content.is_empty()
                || !m.tool_results.is_empty()
                || m.result.is_some()
                || m.has_pending_user_input == Some(true)
        })
        .map(|m| if query.summary { strip_heavy_fields(m) } else { m })
        .collect();
    let total_count = ui_messages.len();

    Ok(Json(GetMessagesResponse {
        messages: ui_messages,
        total_count,
    }))
}

/// Strip heavy fields from a message for summary/CLI consumption.
fn strip_heavy_fields(mut msg: UiMessage) -> UiMessage {
    // Remove tool_input from content blocks
    for block in &mut msg.content {
        block.tool_input = None;
    }
    // Truncate tool result content and remove typed_result
    for tr in &mut msg.tool_results {
        tr.typed_result = None;
        if let Some(ref content) = tr.content {
            if content.len() > 200 {
                tr.content = Some(format!("{}…", &content[..200]));
            }
        }
    }
    // Remove SDK metadata
    msg.sdk_metadata = None;
    msg
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
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_pending_messages(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<GetPendingMessagesResponse>, MessageError> {
    let pending = state.messages.get_pending_user(&task_id).await?;
    let last_result = state.messages.get_last_result(&task_id).await?;

    let mut messages: Vec<PendingMessageItem> = pending
        .iter()
        .map(|m| PendingMessageItem {
            uuid: m.uuid.clone(),
            message_type: m.message_type,
            timestamp: m.timestamp.clone(),
            message_content: Some(m.message_content.clone()),
            agent_mode: m.agent_mode.clone(),
            model: m.model.clone(),
            resume: m.resume,
            will_not_resume: None,
        })
        .collect();

    if let Some(result) = last_result {
        messages.push(PendingMessageItem {
            uuid: result.uuid.clone(),
            message_type: result.message_type,
            timestamp: result.timestamp.clone(),
            message_content: None,
            agent_mode: None,
            model: None,
            resume: true,
            will_not_resume: result.will_not_resume,
        });
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
        (status = 404, description = "Message not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_queued_message(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path((task_id, message_id)): Path<(String, String)>,
) -> Result<Json<DeleteMessageResponse>, MessageError> {
    let deleted = state.messages.delete_queued(&task_id, &message_id).await?;
    if !deleted {
        return Err(MessageError::NotFound(message_id));
    }

    tracing::info!(task_id = %task_id, message_id = %message_id, "Queued message deleted");

    Ok(Json(DeleteMessageResponse {
        status: "deleted".to_string(),
    }))
}
