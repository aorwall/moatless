//! Task API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::{connections_handlers, handlers};
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
        handlers::mark_task_read,
        handlers::follow_task,
        handlers::unfollow_task,
        handlers::close_task,
        handlers::reopen_task,
        handlers::start_task,
        handlers::stop_task,
        handlers::set_task_tags,
        handlers::update_visibility,
        handlers::update_ttl,
        connections_handlers::list_task_connections,
        connections_handlers::get_task_connections,
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
        .routes(routes!(handlers::reopen_task))
        .routes(routes!(handlers::mark_task_read))
        .routes(routes!(handlers::follow_task, handlers::unfollow_task))
        .routes(routes!(handlers::start_task))
        .routes(routes!(handlers::stop_task))
        .routes(routes!(handlers::set_task_tags))
        .routes(routes!(handlers::update_visibility))
        .routes(routes!(handlers::update_ttl))
        .routes(routes!(connections_handlers::list_task_connections))
        .routes(routes!(connections_handlers::get_task_connections))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
