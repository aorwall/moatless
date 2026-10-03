//! Where the GitHub API proxy mounts.

use axum::Router;
use axum::routing::any;

use super::handlers;
use crate::state::AppState;

/// `/github/{api|uploads}/…`, which a sandbox's `gh` reaches GitHub through.
///
/// Mounted outside `/api`, and exempt from the auth middleware
/// (`moatless_identity::auth::middleware`): the route verifies the pod's task
/// token itself and resolves the acting identity from the sandbox it names.
pub fn github_proxy_router(state: AppState) -> Router {
    Router::new()
        .route("/github/{*rest}", any(handlers::proxy))
        .with_state(state)
}
