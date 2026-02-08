//! Task CRUD module.

pub mod error;
pub mod handlers;
pub mod types;

use axum::routing::{delete, get, patch, post, put};
use axum::Router;

use crate::state::AppState;

/// Build the task router.
pub fn task_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/tasks", post(handlers::create_task))
        .route("/api/v1/tasks/vibe", post(handlers::create_vibe_task))
        .route("/api/v1/tasks", get(handlers::list_tasks))
        .route("/api/v1/tasks/{task_id}", get(handlers::get_task))
        .route("/api/v1/tasks/{task_id}", patch(handlers::update_task_metadata))
        .route("/api/v1/tasks/{task_id}", delete(handlers::delete_task))
        .route("/api/v1/tasks/{task_id}/close", post(handlers::close_task))
        .route("/api/v1/tasks/{task_id}/start", post(handlers::start_task))
        .route("/api/v1/tasks/{task_id}/tags", put(handlers::set_task_tags))
        .route("/api/v1/tasks/{task_id}/visibility", put(handlers::update_visibility))
}
