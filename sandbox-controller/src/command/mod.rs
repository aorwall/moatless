//! Command management module.

pub mod defaults;
pub mod error;
pub mod handlers;
pub mod types;

use axum::routing::{delete, get, patch, post};
use axum::Router;

use crate::state::AppState;

/// Build the command router.
pub fn command_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/commands", post(handlers::create_command))
        .route("/api/v1/commands", get(handlers::list_commands))
        .route("/api/v1/commands/{name}", get(handlers::get_command))
        .route("/api/v1/commands/{name}", patch(handlers::update_command))
        .route("/api/v1/commands/{name}", delete(handlers::delete_command))
        .route(
            "/api/v1/tasks/{task_id}/commands",
            get(handlers::get_available_commands),
        )
}
