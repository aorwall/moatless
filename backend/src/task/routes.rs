//! Task API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "tasks", description = "Task CRUD operations and lifecycle"),
    ),
    paths(
        handlers::create_task,
        handlers::list_tasks,
        handlers::get_task,
        handlers::update_task_metadata,
        handlers::delete_task,
        handlers::close_task,
        handlers::start_task,
        handlers::stop_task,
        handlers::set_task_tags,
        handlers::update_visibility,
        handlers::update_ttl,
    )
)]
pub struct TaskApiDoc;

/// Build the task router with OpenAPI metadata.
pub fn task_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(TaskApiDoc::openapi())
        .routes(routes!(handlers::create_task, handlers::list_tasks))
        .routes(routes!(
            handlers::get_task,
            handlers::update_task_metadata,
            handlers::delete_task
        ))
        .routes(routes!(handlers::close_task))
        .routes(routes!(handlers::start_task))
        .routes(routes!(handlers::stop_task))
        .routes(routes!(handlers::set_task_tags))
        .routes(routes!(handlers::update_visibility))
        .routes(routes!(handlers::update_ttl))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
