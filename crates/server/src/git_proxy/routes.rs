//! Where the git smart-HTTP proxy mounts.

use axum::Router;
use axum::routing::{get, post};

use super::handlers;
use crate::state::AppState;

/// Mounted outside `/api`, and exempt from the auth middleware
/// (`moatless_identity::auth::middleware`): git authenticates with HTTP Basic
/// and the route reads the task token out of it.
pub fn git_proxy_router(state: AppState) -> Router {
    Router::new()
        .route("/git/{repository_id}/info/refs", get(handlers::info_refs))
        .route(
            "/git/{repository_id}/git-upload-pack",
            post(handlers::upload_pack),
        )
        .route(
            "/git/{repository_id}/git-receive-pack",
            post(handlers::receive_pack),
        )
        .with_state(state)
}
