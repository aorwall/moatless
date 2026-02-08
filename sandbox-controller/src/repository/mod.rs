//! Repository CRUD module.

pub mod error;
pub mod handlers;
pub mod types;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::state::AppState;

/// Build the repository router.
pub fn repository_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/repositories", post(handlers::create_repository))
        .route("/api/v1/repositories", get(handlers::list_repositories))
        .route(
            "/api/v1/repositories/{repository_id}",
            get(handlers::get_repository),
        )
        .route(
            "/api/v1/repositories/{repository_id}",
            put(handlers::update_repository),
        )
        .route(
            "/api/v1/repositories/{repository_id}",
            delete(handlers::delete_repository),
        )
        .route(
            "/api/v1/repositories/{repository_id}/convert-to-template",
            post(handlers::convert_to_template),
        )
        .route(
            "/api/v1/repositories/{repository_id}/branches",
            get(handlers::get_branches),
        )
}
