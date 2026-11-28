//! Feedback HTTP handlers.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};

use crate::task::operations::TaskCommands;
use moatless_core::http_auth::AuthUser;
use moatless_workspace_ports::RepositoryReader;

use super::error::{FeedbackError, UnknownFeedbackDiscriminant};
use super::models::{Feedback, FeedbackFilter, FeedbackWithTask};
use super::operations::FeedbackNotes;
use super::schemas::{
    CreateFeedbackRequest, FeedbackListItem, FeedbackResponse, ListAllFeedbackQuery,
    ListAllFeedbackResponse, ListFeedbackResponse, UpdateFeedbackStatusRequest,
};
use moatless_types::feedback::{FeedbackCategory, FeedbackSource, FeedbackStatus, FeedbackSurface};

/// Page size when the caller names none, and the ceiling a larger one is
/// clamped to.
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

/// Create a new feedback row scoped to a task (and optionally a message or tool
/// call within that task).
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
    State(tasks): State<Arc<TaskCommands>>,
    State(feedback): State<Arc<FeedbackNotes>>,
    auth: AuthUser,
    Json(req): Json<CreateFeedbackRequest>,
) -> Result<Json<FeedbackResponse>, FeedbackError> {
    let source = if auth.is_agent_context {
        FeedbackSource::Agent
    } else {
        FeedbackSource::User
    };
    let row = super::service::create_feedback(
        tasks.as_ref(),
        feedback.as_ref(),
        &auth.scope_context(),
        source,
        req,
    )
    .await?;
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
    State(tasks): State<Arc<TaskCommands>>,
    State(feedback): State<Arc<FeedbackNotes>>,
    auth: AuthUser,
    Path(feedback_id): Path<String>,
    Json(req): Json<UpdateFeedbackStatusRequest>,
) -> Result<Json<FeedbackResponse>, FeedbackError> {
    let row = super::service::set_feedback_status(
        tasks.as_ref(),
        feedback.as_ref(),
        &auth.scope_context(),
        feedback_id,
        req,
    )
    .await?;
    Ok(Json(to_response(&row)?))
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
    State(tasks): State<Arc<TaskCommands>>,
    State(feedback): State<Arc<FeedbackNotes>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<ListFeedbackResponse>, FeedbackError> {
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

/// List feedback across every task the caller may read, newest first.
///
/// The per-task endpoint answers "does this message have feedback"; this one
/// answers "what has the fleet been told" — filterable by workspace,
/// repository, source, surface, category, submitter, and age.
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
    State(feedback): State<Arc<FeedbackNotes>>,
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
        .filter_map(|r| r.repository_id.clone())
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
    if let Ok(date) = raw.parse::<chrono::NaiveDate>()
        && let Some(midnight) = date.and_hms_opt(0, 0, 0)
    {
        return Ok(DateTime::from_naive_utc_and_offset(midnight, Utc));
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
        repository_name: row
            .repository_id
            .as_ref()
            .and_then(|id| repo_names.get(id))
            .cloned(),
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
