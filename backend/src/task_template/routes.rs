//! Task template API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "task-templates", description = "Task template CRUD operations"),
    ),
    paths(
        handlers::create_task_template,
        handlers::list_task_templates,
        handlers::list_featured_task_templates,
        handlers::get_task_template,
        handlers::update_task_template,
        handlers::delete_task_template,
    )
)]
pub struct TaskTemplateApiDoc;

/// Build the task template router with OpenAPI metadata.
pub fn task_template_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(TaskTemplateApiDoc::openapi())
        .routes(routes!(
            handlers::create_task_template,
            handlers::list_task_templates
        ))
        .routes(routes!(handlers::list_featured_task_templates))
        .routes(routes!(
            handlers::get_task_template,
            handlers::update_task_template,
            handlers::delete_task_template
        ))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
