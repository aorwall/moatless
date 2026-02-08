//! Message retrieval and transformation module.

pub mod error;
pub mod handlers;
pub mod transform;
pub mod types;

use axum::routing::{delete, get};
use axum::Router;

use crate::state::AppState;

/// Build the message router.
pub fn message_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/tasks/{task_id}/messages",
            get(handlers::get_messages),
        )
        .route(
            "/api/v1/tasks/{task_id}/messages/pending",
            get(handlers::get_pending_messages),
        )
        .route(
            "/api/v1/tasks/{task_id}/messages/{message_id}",
            delete(handlers::delete_queued_message),
        )
}
