//! Task-connections API routes with OpenAPI documentation.
//!
//! The two operations keep `tag = "tasks"`. The OpenAPI tag is the external
//! contract that drives the frontend's tags-split client codegen, so these
//! operations stay grouped under `tasks` even though the handlers live here.
//! This `ApiDoc` declares no tag of its own; `task::routes::TaskApiDoc` owns the
//! `tasks` tag description.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    paths(
        handlers::list_task_connections,
        handlers::get_task_connections,
    )
)]
pub struct TaskConnectionsApiDoc;

/// Build the task-connections router with OpenAPI metadata.
pub fn task_connections_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(TaskConnectionsApiDoc::openapi())
        .routes(routes!(handlers::list_task_connections))
        .routes(routes!(handlers::get_task_connections))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
