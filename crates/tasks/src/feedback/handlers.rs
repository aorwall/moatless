//! Feedback HTTP handlers.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};

use crate::task::dao::TaskDao;
use moatless_core::http_auth::AuthUser;
use moatless_ports::workspace::RepositoryReader;

use super::dao::{FeedbackDao, FeedbackFilter};
use super::error::{FeedbackError, UnknownFeedbackDiscriminant};
use super::models::{Feedback, FeedbackWithTask};
use super::schemas::{
    CreateFeedbackRequest, FeedbackListItem, FeedbackResponse, ListAllFeedbackQuery,
    ListAllFeedbackResponse, ListFeedbackResponse, UpdateFeedbackStatusRequest,
};
use moatless_types::feedback::{FeedbackCategory, FeedbackSource, FeedbackStatus, FeedbackSurface};

/// Page size when the caller names none, and the ceiling a larger one is
/// clamped to.
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

/// Longest status note accepted. A note says why a status was set; the
/// argument for it belongs in the issue the status points at.
const MAX_STATUS_NOTE: usize = 2_000;

/// Create a new feedback row scoped to a task (and optionally a message
/// or tool call within that task).
#[utoipa::path(
    post,
    path = "/api/v1/feedback",
    tag = "feedback",
    request_body = CreateFeedbackRequest,
    responses(
        (status = 200, description = "Feedback created", body = FeedbackResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn create_feedback(
    State(tasks): State<Arc<dyn TaskDao>>,
    State(feedback): State<Arc<dyn FeedbackDao>>,
    auth: AuthUser,
    Json(req): Json<CreateFeedbackRequest>,
) -> Result<Json<FeedbackResponse>, FeedbackError> {
    // Validate scope: tool_use_id requires message_uuid.
    if req.tool_use_id.is_some() && req.message_uuid.is_none() {
        return Err(FeedbackError::BadRequest(
            "tool_use_id requires message_uuid to be set".to_string(),
        ));
    }

    // Validate scope is consistent with surface.
    match (
        req.surface,
        req.message_uuid.is_some(),
        req.tool_use_id.is_some(),
    ) {
        (FeedbackSurface::Task, false, false) => {}
        (FeedbackSurface::Message, true, false) => {}
        (FeedbackSurface::ToolCall, true, true) => {}
        (FeedbackSurface::PreviewFailure, _, _) => {}
        (surface, _, _) => {
            return Err(FeedbackError::BadRequest(format!(
                "surface={surface} does not match the provided scope (message_uuid, tool_use_id)"
            )));
        }
    }

    if req.comment.trim().is_empty() {
        return Err(FeedbackError::BadRequest(
            "comment must not be empty".to_string(),
        ));
    }
    if req.comment.len() > 8_000 {
        return Err(FeedbackError::BadRequest(
            "comment too long (max 8000)".to_string(),
        ));
    }

    // Verify task exists and the caller has access.
    let _task = tasks
        .find_by_id(&req.task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| FeedbackError::TaskNotFound(req.task_id.clone()))?;

    // A caller who names no message asked for task-level feedback, but the
    // common case — the agent running `moat feedback` with no flags — means
    // "about what I am doing right now", not "about the task as a whole".
    // Best effort: point it at the task's latest activity instead. A caller
    // that did name a message (or asked for `preview_failure`) keeps exactly
    // what it sent.
    let (message_uuid, tool_use_id, surface) =
        if req.surface == FeedbackSurface::Task && req.message_uuid.is_none() {
            match feedback.latest_activity(&req.task_id).await? {
                Some(activity) if activity.tool_use_id.is_some() => (
                    Some(activity.message_uuid),
                    activity.tool_use_id,
                    FeedbackSurface::ToolCall,
                ),
                Some(activity) => (Some(activity.message_uuid), None, FeedbackSurface::Message),
                None => (None, None, FeedbackSurface::Task),
            }
        } else {
            (
                req.message_uuid.clone(),
                req.tool_use_id.clone(),
                req.surface,
            )
        };

    let source = if auth.is_agent_context {
        FeedbackSource::Agent
    } else {
        FeedbackSource::User
    };

    let row = Feedback {
        feedback_id: uuid::Uuid::new_v4().to_string(),
        task_id: req.task_id.clone(),
        message_uuid,
        tool_use_id,
        source: source.as_str().to_string(),
        surface: surface.as_str().to_string(),
        category: req.category.map(|c| c.as_str().to_string()),
        comment: req.comment.clone(),
        metadata: req.metadata.clone(),
        created_by: auth.user_id.clone().to_string(),
        created_at: Utc::now(),
        status: FeedbackStatus::Open.as_str().to_string(),
        status_note: None,
        status_updated_by: None,
        status_updated_at: None,
    };

    feedback.insert(&row).await?;
    tracing::info!(
        task_id = %row.task_id,
        feedback_id = %row.feedback_id,
        surface = %row.surface,
        source = %row.source,
        category = ?row.category,
        message_uuid = ?row.message_uuid,
        tool_use_id = ?row.tool_use_id,
        "Feedback recorded"
    );

    Ok(Json(to_response(&row)?))
}

/// Set a note's triage status. The row records who set it, so a note closed by
/// the same context that filed it stays visible as such.
#[utoipa::path(
    patch,
    path = "/api/v1/feedback/{feedback_id}/status",
    tag = "feedback",
    params(
        ("feedback_id" = String, Path, description = "Feedback identifier")
    ),
    request_body = UpdateFeedbackStatusRequest,
    responses(
        (status = 200, description = "Status set", body = FeedbackResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Feedback not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn set_feedback_status(
    State(tasks): State<Arc<dyn TaskDao>>,
    State(feedback): State<Arc<dyn FeedbackDao>>,
    auth: AuthUser,
    Path(feedback_id): Path<String>,
    Json(req): Json<UpdateFeedbackStatusRequest>,
) -> Result<Json<FeedbackResponse>, FeedbackError> {
    let note = req
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty())
        .map(str::to_string);
    if note
        .as_deref()
        .is_some_and(|note| note.len() > MAX_STATUS_NOTE)
    {
        return Err(FeedbackError::BadRequest(format!(
            "note too long (max {MAX_STATUS_NOTE})"
        )));
    }

    let row = feedback
        .find_by_id(&feedback_id)
        .await?
        .ok_or_else(|| FeedbackError::FeedbackNotFound(feedback_id.clone()))?;

    // Access control reads the note's task: `feedback` carries no scope columns
    // of its own, so a row is writable exactly when its task is readable.
    tasks
        .find_by_id(&row.task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| FeedbackError::FeedbackNotFound(feedback_id.clone()))?;

    let updated = feedback
        .update_status(
            &feedback_id,
            req.status.as_str(),
            note,
            &auth.user_id.clone().to_string(),
        )
        .await?
        .ok_or_else(|| FeedbackError::FeedbackNotFound(feedback_id.clone()))?;

    tracing::info!(
        task_id = %updated.task_id,
        feedback_id = %updated.feedback_id,
        status = %updated.status,
        status_updated_by = ?updated.status_updated_by,
        "Feedback status set"
    );

    Ok(Json(to_response(&updated)?))
}

/// List all feedback rows for a task, ordered oldest → newest. Used by the
/// frontend to render the "has feedback" indicator on messages and tool calls.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/feedback",
    tag = "feedback",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Feedback list", body = ListFeedbackResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_task_feedback(
    State(tasks): State<Arc<dyn TaskDao>>,
    State(feedback): State<Arc<dyn FeedbackDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<ListFeedbackResponse>, FeedbackError> {
    // Access check via the task lookup.
    let _task = tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| FeedbackError::TaskNotFound(task_id.clone()))?;

    let rows = feedback.list_for_task(&task_id).await?;
    let items = rows
        .iter()
        .map(to_response)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ListFeedbackResponse { items }))
}

/// List feedback across every task the caller may read, newest first. The
/// per-task endpoint answers "does this message have feedback"; this one
/// answers "what has the fleet been told", which is why it filters by
/// workspace, repository, source, surface, category, submitter, and age.
#[utoipa::path(
    get,
    path = "/api/v1/feedback",
    tag = "feedback",
    params(ListAllFeedbackQuery),
    responses(
        (status = 200, description = "Feedback list", body = ListAllFeedbackResponse),
        (status = 400, description = "Invalid filter", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_all_feedback(
    State(feedback): State<Arc<dyn FeedbackDao>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    auth: AuthUser,
    Query(query): Query<ListAllFeedbackQuery>,
) -> Result<Json<ListAllFeedbackResponse>, FeedbackError> {
    let since = match query.since.as_deref() {
        Some(raw) => Some(parse_since(raw)?),
        None => None,
    };
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = query.offset.unwrap_or(0).max(0);

    let filter = FeedbackFilter {
        scope: auth.scope_context(),
        workspace_id: query.workspace_id,
        repository_id: query.repository_id,
        task_id: query.task_id,
        source: query.source.map(|s| s.as_str().to_string()),
        surface: query.surface.map(|s| s.as_str().to_string()),
        category: query.category.map(|c| c.as_str().to_string()),
        status: query.status.map(|s| s.as_str().to_string()),
        created_by: query.created_by,
        since,
        limit,
        offset,
    };

    let (rows, total) = feedback.list(&filter).await?;

    let repo_ids: Vec<String> = rows
        .iter()
        .map(|r| r.repository_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let repo_names = repositories.names(&repo_ids).await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "Failed to batch resolve repository names");
        HashMap::new()
    });

    let items = rows
        .iter()
        .map(|row| to_list_item(row, &repo_names))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ListAllFeedbackResponse {
        items,
        total,
        limit,
        offset,
    }))
}

/// Accepts a full RFC 3339 instant or a bare `YYYY-MM-DD`, which is read as
/// midnight UTC on that day.
fn parse_since(raw: &str) -> Result<DateTime<Utc>, FeedbackError> {
    if let Ok(ts) = DateTime::parse_from_rfc3339(raw) {
        return Ok(ts.with_timezone(&Utc));
    }
    if let Ok(date) = raw.parse::<chrono::NaiveDate>() {
        if let Some(midnight) = date.and_hms_opt(0, 0, 0) {
            return Ok(DateTime::from_naive_utc_and_offset(midnight, Utc));
        }
    }
    Err(FeedbackError::BadRequest(format!(
        "since must be RFC 3339 or YYYY-MM-DD, got {raw:?}"
    )))
}

fn to_list_item(
    row: &FeedbackWithTask,
    repo_names: &HashMap<String, String>,
) -> Result<FeedbackListItem, FeedbackError> {
    Ok(FeedbackListItem {
        feedback: to_response(&row.feedback)?,
        task_title: row.task_title.clone(),
        repository_name: repo_names.get(&row.repository_id).cloned(),
        repository_id: row.repository_id.clone(),
        workspace_id: row.workspace_id.clone(),
    })
}

fn to_response(row: &Feedback) -> Result<FeedbackResponse, FeedbackError> {
    // The shared schema crate reports an unknown discriminant as a bare
    // `String`, so the module re-raises it as its own typed error to keep a
    // real source on the chain.
    let source =
        FeedbackSource::try_from(row.source.as_str()).map_err(|_| UnknownFeedbackDiscriminant {
            column: "source",
            value: row.source.clone(),
        })?;
    let surface = FeedbackSurface::try_from(row.surface.as_str()).map_err(|_| {
        UnknownFeedbackDiscriminant {
            column: "surface",
            value: row.surface.clone(),
        }
    })?;
    let category = row
        .category
        .as_deref()
        .map(FeedbackCategory::try_from)
        .transpose()
        .map_err(|_| UnknownFeedbackDiscriminant {
            column: "category",
            value: row.category.clone().unwrap_or_default(),
        })?;
    let status =
        FeedbackStatus::try_from(row.status.as_str()).map_err(|_| UnknownFeedbackDiscriminant {
            column: "status",
            value: row.status.clone(),
        })?;
    Ok(FeedbackResponse {
        feedback_id: row.feedback_id.clone(),
        task_id: row.task_id.clone(),
        message_uuid: row.message_uuid.clone(),
        tool_use_id: row.tool_use_id.clone(),
        source,
        surface,
        category,
        comment: row.comment.clone(),
        metadata: row.metadata.clone(),
        created_by: row.created_by.clone(),
        created_at: row
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        status,
        status_note: row.status_note.clone(),
        status_updated_by: row.status_updated_by.clone(),
        status_updated_at: row
            .status_updated_at
            .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
    })
}
