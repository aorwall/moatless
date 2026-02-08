//! Agent execution module — handles message sending, command execution, and agent lifecycle.

pub mod error;
pub mod handlers;
pub mod notify;
pub mod orchestration;
pub mod request_builder;
pub mod types;

use axum::routing::post;
use axum::Router;

use crate::state::AppState;

/// Build the agent router.
pub fn agent_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/tasks/{task_id}/messages",
            post(handlers::send_message),
        )
        .route(
            "/api/v1/tasks/{task_id}/commands",
            post(handlers::send_command),
        )
        .route(
            "/api/v1/tasks/{task_id}/stop",
            post(handlers::stop_agent),
        )
}
