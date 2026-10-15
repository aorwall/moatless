//! Guided-review HTTP handlers, called by `moat review-guide`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use crate::task::operations::TaskCommands;
use moatless_core::http_auth::AuthUser;

use super::error::ReviewGuideError;
use super::operations::ReviewGuides;
use super::schemas::{ReviewGuideRunResponse, SubmitReviewGuideRequest, SubmitReviewGuideResponse};

/// The pull request and commit range a Task was started to write a guide for.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/review-guide",
    tag = "review-guides",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "The run the task is writing", body = ReviewGuideRunResponse),
        (status = 404, description = "Task not found, or not writing a guide", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_review_guide_run(
    State(tasks): State<Arc<TaskCommands>>,
    State(guides): State<Arc<ReviewGuides>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<ReviewGuideRunResponse>, ReviewGuideError> {
    let run =
        super::service::run_for_task(&tasks, &guides, &auth.scope_context(), &task_id).await?;
    Ok(Json(run))
}

/// Submit the guide a Task wrote. The server holds it to the change and stores
/// what is left as the pull request's guide.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/review-guide",
    tag = "review-guides",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = SubmitReviewGuideRequest,
    responses(
        (status = 200, description = "Guide stored", body = SubmitReviewGuideResponse),
        (status = 400, description = "Guide refused", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found, or not writing a guide", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn submit_review_guide(
    State(tasks): State<Arc<TaskCommands>>,
    State(guides): State<Arc<ReviewGuides>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SubmitReviewGuideRequest>,
) -> Result<Json<SubmitReviewGuideResponse>, ReviewGuideError> {
    let response =
        super::service::submit(&tasks, &guides, &auth.scope_context(), &task_id, req).await?;
    Ok(Json(response))
}
