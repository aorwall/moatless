//! Feedback API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "feedback", description = "Qualitative feedback on agent runs"),
    ),
    paths(
        handlers::create_feedback,
        handlers::list_task_feedback,
    )
)]
pub struct FeedbackApiDoc;

/// Build the feedback router with OpenAPI metadata.
pub fn feedback_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(FeedbackApiDoc::openapi())
        .routes(routes!(handlers::create_feedback))
        .routes(routes!(handlers::list_task_feedback))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
