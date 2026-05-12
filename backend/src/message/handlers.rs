//! Message HTTP handlers.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;

use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::MessageError;
use super::schemas::*;
use super::send;
use super::transform;

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
    let _task = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
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
            Some(&auth.user_id),
        )
        .await?;
    } else {
        // Regular message or command — frontend sends the prompt text directly.
        // command_name is stored on the message for display purposes only.
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
            req.command_name.as_deref(),
            req.source_task_id.as_deref(),
            req.start_fresh.unwrap_or(false),
            Some(&auth.user_id),
        )
        .await?;
    }

    // Ensure the sandbox is running (auto-provisions if needed) and notify it.
    if let Err(error_msg) =
        crate::sandbox::notify::ensure_sandbox_and_notify(&state, &task_id).await
    {
        send::persist_error_result(&state, &task_id, &error_msg).await;
    }

    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Message sent to agent");

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(Json(SendMessageResponse {
        status: "sent".to_string(),
        timestamp: now,
    }))
}

/// Default page size when `limit` is not specified.
const DEFAULT_MESSAGE_LIMIT: i32 = 50;
/// Maximum allowed page size.
const MAX_MESSAGE_LIMIT: i32 = 200;

/// Tool calls whose `tool_input` must survive `?summary=true` projection
/// because the frontend renders it directly (no expand-on-demand fetch).
/// Keep in sync with `transform::check_pending_user_input` and
/// `apps/frontend/src/features/events/utils/event-handlers.ts`.
const INTERACTIVE_TOOL_NAMES: &[&str] = &["ExitPlanMode", "AskUserQuestion"];

/// Query parameters for GET /messages.
#[derive(Debug, Deserialize)]
pub struct GetMessagesQuery {
    /// When true, strip heavy fields (tool_input, typed_result, sdk_metadata)
    /// and truncate tool result content. Useful for CLI/summary views.
    #[serde(default)]
    pub summary: bool,
    /// Cursor: return only rows strictly older than this message id.
    /// Omit for the most recent page.
    #[serde(default)]
    pub before_id: Option<i32>,
    /// Page size. Defaults to 50, clamped to 1..=200.
    #[serde(default)]
    pub limit: Option<i32>,
}

/// Get a chronological page of messages for a task.
///
/// Without query params returns the most recent `DEFAULT_MESSAGE_LIMIT` rows.
/// Pass `before_id=<oldest_in_previous_page>` to fetch older pages.
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
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_messages(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<GetMessagesQuery>,
) -> Result<Json<GetMessagesResponse>, MessageError> {
    ensure_task_read_access(&state, &auth, &task_id).await?;

    let limit = query
        .limit
        .unwrap_or(DEFAULT_MESSAGE_LIMIT)
        .clamp(1, MAX_MESSAGE_LIMIT);

    let (messages, contexts, blocks) = state
        .messages
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
        state
            .files
            .find_by_ids(&legacy_file_ids)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Failed to batch-load file metadata");
                std::collections::HashMap::new()
            })
    };

    let ui_messages: Vec<UiMessage> =
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

    let total_count = ui_messages.len();
    Ok(Json(GetMessagesResponse {
        messages: ui_messages,
        next_before_id,
        has_more,
        total_count,
    }))
}

/// Strip heavy fields from a message for summary/CLI consumption.
///
/// Keeps a small preview of `tool_input` so the collapsed UI row can render the
/// same compact "(arg)" suffix it does today, but without shipping full
/// `Edit`/`Write` payloads or full `Bash` results across the wire on every
/// list/SSE refresh.
fn strip_heavy_fields(mut msg: UiMessage) -> UiMessage {
    for block in &mut msg.content {
        if let Some(input) = block.tool_input.take() {
            let keep_full = block
                .tool_name
                .as_deref()
                .is_some_and(|n| INTERACTIVE_TOOL_NAMES.contains(&n));
            block.tool_input = Some(if keep_full {
                input
            } else {
                tool_input_preview(&input)
            });
        }
    }
    // Truncate tool result content and remove typed_result
    for tr in &mut msg.tool_results {
        tr.typed_result = None;
        if let Some(ref content) = tr.content {
            if content.len() > 200 {
                tr.content = Some(truncate_with_ellipsis(content, 200));
            }
        }
    }
    // Remove SDK metadata
    msg.sdk_metadata = None;
    msg
}

/// Build a compact preview from a `tool_input` JSON value: take the first
/// (key, value) entry, stringify non-string values, and truncate strings to
/// 100 chars (char-boundary safe). Returns the original value if it is not
/// an object, or an empty object if it is empty.
fn tool_input_preview(value: &serde_json::Value) -> serde_json::Value {
    let Some(obj) = value.as_object() else {
        return value.clone();
    };

    let Some((key, raw_value)) = obj.iter().next() else {
        return serde_json::Value::Object(serde_json::Map::new());
    };

    let display = match raw_value {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    let truncated = truncate_with_ellipsis(&display, 100);

    let mut out = serde_json::Map::with_capacity(1);
    out.insert(key.clone(), serde_json::Value::String(truncated));
    serde_json::Value::Object(out)
}

/// Truncate a string at a UTF-8 char boundary, appending `…` when shortened.
fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
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
        (status = 404, description = "Tool call not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_tool_call_detail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((task_id, tool_use_id)): Path<(String, String)>,
) -> Result<Json<ToolCallDetailResponse>, MessageError> {
    ensure_task_read_access(&state, &auth, &task_id).await?;

    let row = state
        .messages
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

#[cfg(test)]
mod preview_tests {
    use super::tool_input_preview;
    use serde_json::json;

    #[test]
    fn scalar_value_is_stringified() {
        let preview = tool_input_preview(&json!({ "n": 42 }));
        assert_eq!(preview, json!({ "n": "42" }));
    }

    #[test]
    fn long_string_is_truncated_with_ellipsis() {
        let long = "x".repeat(150);
        let preview = tool_input_preview(&json!({ "content": long }));
        let value = preview["content"].as_str().unwrap();
        assert_eq!(value.chars().count(), 101); // 100 chars + "…"
        assert!(value.ends_with('…'));
    }

    #[test]
    fn nested_object_is_stringified() {
        let preview = tool_input_preview(&json!({ "todos": [{ "id": 1 }, { "id": 2 }] }));
        let value = preview["todos"].as_str().unwrap();
        assert!(value.starts_with('['));
    }

    #[test]
    fn empty_input_returns_empty_object() {
        let preview = tool_input_preview(&json!({}));
        assert_eq!(preview, json!({}));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        // Multi-byte chars: "é" repeated past the limit must not split a code point.
        let s = "é".repeat(150);
        let preview = tool_input_preview(&json!({ "k": s }));
        let value = preview["k"].as_str().unwrap();
        assert_eq!(value.chars().count(), 101);
    }
}

#[cfg(test)]
mod strip_heavy_fields_tests {
    use super::strip_heavy_fields;
    use moatless_api_schemas::enums::{MessageRole, MessageType};
    use moatless_api_schemas::message::{UiContentBlock, UiMessage};
    use serde_json::{json, Value};

    fn ui_message_with_tool_use(tool_name: &str, tool_input: Value) -> UiMessage {
        UiMessage {
            uuid: "msg-1".to_string(),
            message_type: MessageType::Assistant,
            role: MessageRole::Assistant,
            timestamp: "2026-05-04T00:00:00.000Z".to_string(),
            session_id: None,
            turn_number: None,
            agent_mode: None,
            model: None,
            is_queued: None,
            content: vec![UiContentBlock {
                block_type: "tool_use".to_string(),
                text: None,
                tool_use_id: Some("toolu_1".to_string()),
                tool_name: Some(tool_name.to_string()),
                tool_input: Some(tool_input),
                show_in_ui: None,
            }],
            contexts: vec![],
            tool_results: vec![],
            has_pending_user_input: None,
            message_id: None,
            stop_reason: None,
            stop_sequence: None,
            usage: None,
            result: None,
            sdk_metadata: None,
            external_source: None,
            command_name: None,
            parent_uuid: None,
            source_task_id: None,
            sent_by: None,
        }
    }

    #[test]
    fn strip_heavy_fields_keeps_exit_plan_mode_input_verbatim() {
        let plan = "## Long markdown\n".to_string() + &"detail line. ".repeat(40);
        assert!(plan.chars().count() > 200);
        let input = json!({
            "allowedPrompts": [
                { "prompt": "do this", "tool": "Bash" },
                { "prompt": "do that", "tool": "Edit" },
            ],
            "plan": plan,
        });

        let stripped = strip_heavy_fields(ui_message_with_tool_use("ExitPlanMode", input.clone()));

        let kept = stripped.content[0]
            .tool_input
            .as_ref()
            .expect("tool_input preserved");
        assert_eq!(kept, &input);
        assert_eq!(kept["plan"].as_str().unwrap(), plan);
    }

    #[test]
    fn strip_heavy_fields_keeps_ask_user_question_input_verbatim() {
        let input = json!({
            "questions": [
                {
                    "question": "Pick a color",
                    "header": "Style",
                    "multiSelect": false,
                    "options": [
                        { "label": "Red" },
                        { "label": "Blue" },
                    ],
                }
            ]
        });

        let stripped =
            strip_heavy_fields(ui_message_with_tool_use("AskUserQuestion", input.clone()));

        assert_eq!(
            stripped.content[0]
                .tool_input
                .as_ref()
                .expect("tool_input preserved"),
            &input
        );
    }

    #[test]
    fn strip_heavy_fields_still_previews_bash() {
        let command = "echo ".to_string() + &"x".repeat(500);
        let input = json!({ "command": command });

        let stripped = strip_heavy_fields(ui_message_with_tool_use("Bash", input));

        let kept = stripped.content[0]
            .tool_input
            .as_ref()
            .expect("tool_input present");
        let obj = kept.as_object().expect("object");
        assert_eq!(obj.len(), 1);
        let preview = obj
            .get("command")
            .and_then(Value::as_str)
            .expect("command preview");
        assert!(preview.chars().count() <= 101);
        assert!(preview.ends_with('…'));
    }
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
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<GetPendingMessagesResponse>, MessageError> {
    ensure_task_read_access(&state, &auth, &task_id).await?;

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
    auth: AuthUser,
    Path((task_id, message_id)): Path<(String, String)>,
) -> Result<Json<DeleteMessageResponse>, MessageError> {
    ensure_task_read_access(&state, &auth, &task_id).await?;

    let deleted = state.messages.delete_queued(&task_id, &message_id).await?;
    if !deleted {
        return Err(MessageError::NotFound(message_id));
    }

    tracing::info!(task_id = %task_id, message_id = %message_id, "Queued message deleted");

    Ok(Json(DeleteMessageResponse {
        status: "deleted".to_string(),
    }))
}

async fn ensure_task_read_access(
    state: &AppState,
    auth: &AuthUser,
    task_id: &str,
) -> Result<(), MessageError> {
    state
        .tasks
        .find_by_id(task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| MessageError::NotFound(task_id.to_string()))?;
    Ok(())
}
