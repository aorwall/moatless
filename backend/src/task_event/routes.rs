//! Task event API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "task-events", description = "Task event history"),
    ),
    paths(
        handlers::list_task_events,
    )
)]
pub struct TaskEventApiDoc;

/// Build the task event router with OpenAPI metadata.
pub fn task_event_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(TaskEventApiDoc::openapi())
        .routes(routes!(handlers::list_task_events))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
