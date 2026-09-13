//! Message HTTP handlers.

use super::error::MessageError;
use super::schemas::*;
use super::service;
use super::summary::strip_heavy_fields;
use super::transform;
use crate::message::operations::Messages;
use crate::message::send::MessageSendService;
use crate::task::operations::TaskCommands;
use axum::Json;
use axum::extract::{Path, Query, State};
use moatless_core::file_storage::Files;
use moatless_core::http_auth::AuthUser;
use moatless_ports::sandbox::SandboxWaker;
use serde::Deserialize;
use std::sync::Arc;

/// Send a message to the agent for a task.
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "Task is closed", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn send_message(
    State(tasks): State<Arc<TaskCommands>>,
    State(message_send): State<Arc<MessageSendService>>,
    State(sandbox_waker): State<Arc<dyn SandboxWaker>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, MessageError> {
    // A tool response needs both halves; one alone is a plain message.
    let tool_response = match (req.tool_use_id, req.tool_name) {
        (Some(tool_use_id), Some(tool_name)) => Some(service::ToolResponse {
            tool_use_id,
            tool_name,
            payload: req.tool_response,
        }),
        _ => None,
    };

    let user_id = auth.user_id.to_string();
    let message_uuid = service::send_user_message(
        tasks.as_ref(),
        message_send.as_ref(),
        sandbox_waker.as_ref(),
        &service::Sender {
            user_id: &user_id,
            scope: &auth.scope_context(),
            is_agent_context: auth.is_agent_context,
        },
        &task_id,
        service::UserMessage {
            text: req.message,
            model: req.model,
            agent_mode: req.agent_mode,
            effort: req.effort,
            file_ids: req.file_ids,
            contexts: req.contexts,
            resume: req.resume.unwrap_or(true),
            source_task_id: req.source_task_id,
            start_fresh: req.start_fresh.unwrap_or(false),
            skills: req.skills,
            message_uuid: None,
            tool_response,
        },
    )
    .await?;

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(Json(SendMessageResponse {
        status: "sent".to_string(),
        timestamp: now,
        uuid: message_uuid,
    }))
}

/// Default page size when `limit` is not specified.
const DEFAULT_MESSAGE_LIMIT: i32 = 50;
/// Maximum allowed page size.
const MAX_MESSAGE_LIMIT: i32 = 200;

/// Query parameters for GET /messages.
#[derive(Debug, Deserialize)]
pub struct GetMessagesQuery {
    /// When true, strip heavy fields (`tool_input`, `typed_result`,
    /// `sdk_metadata`) and truncate tool result content. Useful for CLI/summary
    /// views.
    #[serde(default)]
    pub summary: bool,
    /// Cursor: return only rows strictly older than this message id. Omit for
    /// the most recent page.
    #[serde(default)]
    pub before_id: Option<i32>,
    /// Page size. Defaults to 50, clamped to 1..=200.
    #[serde(default)]
    pub limit: Option<i32>,
}

/// Get a chronological page of messages for a task.
///
/// Without query params returns the most recent `DEFAULT_MESSAGE_LIMIT` rows;
/// pass `before_id=<oldest_in_previous_page>` for older pages.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/messages",
    tag = "messages",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("summary" = Option<bool>, Query, description = "Strip heavy fields for summary view"),
        ("before_id" = Option<i32>, Query, description = "Return only rows strictly older than this id"),
        ("limit" = Option<i32>, Query, description = "Page size (default 50, max 200)")
    ),
    responses(
        (status = 200, description = "Page of messages", body = GetMessagesResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_messages(
    State(tasks): State<Arc<TaskCommands>>,
    State(message_dao): State<Arc<Messages>>,
    State(files): State<Arc<Files>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<GetMessagesQuery>,
) -> Result<Json<GetMessagesResponse>, MessageError> {
    ensure_task_read_access(tasks.as_ref(), &auth, &task_id).await?;

    let limit = query
        .limit
        .unwrap_or(DEFAULT_MESSAGE_LIMIT)
        .clamp(1, MAX_MESSAGE_LIMIT);

    let (messages, contexts, blocks) = message_dao
        .get_paginated(&task_id, query.before_id, limit)
        .await?;

    // `has_more` reflects whether the paginated query *filled* the page from
    // the raw row count, not from the post-transform UI message count. The
    // cursor must point at the oldest raw row; otherwise empty-content rows
    // can desync the cursor and produce infinite empty pages.
    let has_more = messages.len() as i32 == limit;
    let next_before_id = if has_more {
        messages.first().map(|m| m.id)
    } else {
        None
    };

    // Pre-fetch file_metadata for the legacy fallback path so messages
    // persisted before `task_message_contexts` was wired up still surface
    // their uploaded files. One batch SELECT covers the whole page.
    let legacy_file_ids: Vec<String> = messages
        .iter()
        .flat_map(super::models::TaskMessage::file_ids)
        .collect();
    let file_metadata = if legacy_file_ids.is_empty() {
        std::collections::HashMap::new()
    } else {
        files
            .metadata_for(&legacy_file_ids)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Failed to batch-load file metadata");
                std::collections::HashMap::new()
            })
    };

    let mut ui_messages: Vec<UiMessage> =
        transform::transform_to_ui_with_files(&messages, &contexts, &blocks, &file_metadata)
            .into_iter()
            .map(|m| {
                if query.summary {
                    strip_heavy_fields(m)
                } else {
                    m
                }
            })
            .collect();

    // Inject `subagentSummary` onto `Agent` blocks via the shared helper — one
    // batched aggregate over the page's `Agent` tool ids. Same source of truth
    // as the live top-level emit.
    super::hooks::hydrate_subagent_summaries_from_dao(&message_dao, &task_id, &mut ui_messages)
        .await;

    let total_count = ui_messages.len();
    Ok(Json(GetMessagesResponse {
        messages: ui_messages,
        next_before_id,
        has_more,
        total_count,
    }))
}

/// Get the full input + result for a single tool call within a task.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/tool-calls/{tool_use_id}",
    tag = "messages",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("tool_use_id" = String, Path, description = "Tool-use block identifier (unique within a task)")
    ),
    responses(
        (status = 200, description = "Tool call detail", body = ToolCallDetailResponse),
        (status = 404, description = "Tool call not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_tool_call_detail(
    State(tasks): State<Arc<TaskCommands>>,
    State(message_dao): State<Arc<Messages>>,
    auth: AuthUser,
    Path((task_id, tool_use_id)): Path<(String, String)>,
) -> Result<Json<ToolCallDetailResponse>, MessageError> {
    ensure_task_read_access(tasks.as_ref(), &auth, &task_id).await?;

    let row = message_dao
        .find_tool_call(&task_id, &tool_use_id)
        .await?
        .ok_or_else(|| MessageError::NotFound(tool_use_id.clone()))?;

    let typed_result = transform::enrich_codex_typed_result(
        &row.tool_name,
        Some(&row.tool_input),
        row.tool_use_result,
    );

    Ok(Json(ToolCallDetailResponse {
        tool_use_id: row.tool_use_id,
        tool_name: row.tool_name,
        tool_input: row.tool_input,
        result_content: row.result_content,
        is_error: row.is_error,
        typed_result,
    }))
}

/// Get the assembled subagent transcript for a single `Agent` tool call.
///
/// Runs the fold-resolving row-transform over rows whose `parent_uuid` equals
/// `tool_use_id`; returned whole, since it's bounded per call and fetched on
/// demand when the `Agent` card expands.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/tool-calls/{tool_use_id}/messages",
    tag = "messages",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("tool_use_id" = String, Path, description = "Spawning Agent tool-use id")
    ),
    responses(
        (status = 200, description = "Subagent transcript", body = SubagentTranscriptResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_subagent_transcript(
    State(tasks): State<Arc<TaskCommands>>,
    State(message_dao): State<Arc<Messages>>,
    auth: AuthUser,
    Path((task_id, tool_use_id)): Path<(String, String)>,
) -> Result<Json<SubagentTranscriptResponse>, MessageError> {
    ensure_task_read_access(tasks.as_ref(), &auth, &task_id).await?;

    let (messages, contexts, blocks) = message_dao
        .get_subagent_messages(&task_id, &tool_use_id)
        .await?;

    let ui_messages =
        transform::transform_subagent_transcript(&messages, &contexts, &blocks, &tool_use_id);

    Ok(Json(SubagentTranscriptResponse {
        messages: ui_messages,
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_pending_messages(
    State(tasks): State<Arc<TaskCommands>>,
    State(message_dao): State<Arc<Messages>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<GetPendingMessagesResponse>, MessageError> {
    ensure_task_read_access(tasks.as_ref(), &auth, &task_id).await?;

    let pending = message_dao.get_pending_user(&task_id).await?;
    let last_result = message_dao.get_last_result(&task_id).await?;

    let mut messages: Vec<PendingMessageItem> = pending
        .iter()
        .map(|m| PendingMessageItem {
            uuid: m.uuid.clone(),
            message_type: m.message_type,
            timestamp: m.timestamp.clone(),
            message_text: m.message_text.clone(),
            agent_mode: m.agent_mode(),
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
            message_text: None,
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

async fn ensure_task_read_access(
    tasks: &TaskCommands,
    auth: &AuthUser,
    task_id: &str,
) -> Result<(), MessageError> {
    tasks
        .find_by_id(task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| MessageError::NotFound(task_id.to_string()))?;
    Ok(())
}
