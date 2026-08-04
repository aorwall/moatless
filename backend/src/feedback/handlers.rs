//! Feedback HTTP handlers.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;

use crate::extractors::AuthUser;
use crate::task::dao::TaskDao;

use super::dao::FeedbackDao;
use super::error::{FeedbackError, UnknownFeedbackDiscriminant};
use super::models::{Feedback, FeedbackSource};
use super::schemas::{
    CreateFeedbackRequest, FeedbackResponse, FeedbackSurface, ListFeedbackResponse,
};

/// Create a new feedback row scoped to a task (and optionally a message
/// or tool call within that task).
#[utoipa::path(
    post,
    path = "/api/v1/feedback",
    tag = "feedback",
    request_body = CreateFeedbackRequest,
    responses(
        (status = 200, description = "Feedback created", body = FeedbackResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorBody),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
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

    let source = if auth.is_agent_context {
        FeedbackSource::Agent
    } else {
        FeedbackSource::User
    };

    let row = Feedback {
        feedback_id: uuid::Uuid::new_v4().to_string(),
        task_id: req.task_id.clone(),
        message_uuid: req.message_uuid.clone(),
        tool_use_id: req.tool_use_id.clone(),
        source: source.as_str().to_string(),
        surface: req.surface.as_str().to_string(),
        comment: req.comment.clone(),
        metadata: req.metadata.clone(),
        created_by: auth.user_id.clone().to_string(),
        created_at: Utc::now(),
    };

    feedback.insert(&row).await?;
    tracing::info!(
        task_id = %row.task_id,
        feedback_id = %row.feedback_id,
        surface = %row.surface,
        source = %row.source,
        message_uuid = ?row.message_uuid,
        tool_use_id = ?row.tool_use_id,
        "Feedback recorded"
    );

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
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
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
    Ok(FeedbackResponse {
        feedback_id: row.feedback_id.clone(),
        task_id: row.task_id.clone(),
        message_uuid: row.message_uuid.clone(),
        tool_use_id: row.tool_use_id.clone(),
        source,
        surface,
        comment: row.comment.clone(),
        metadata: row.metadata.clone(),
        created_by: row.created_by.clone(),
        created_at: row
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    })
}
