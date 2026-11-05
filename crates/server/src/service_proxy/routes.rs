//! Where the service proxy mounts.

use axum::Router;
use axum::routing::any;

use super::handlers;
use crate::state::AppState;

/// `/proxy/{service}/…`, which a pod's service relay forwards to.
///
/// Mounted outside `/api`, and exempt from the auth middleware
/// (`moatless_identity::auth::middleware`): the route verifies the pod's task
/// token itself, as the GitHub proxy does.
pub fn service_proxy_router(state: AppState) -> Router {
    Router::new()
        .route("/proxy/{service}", any(handlers::proxy))
        .route("/proxy/{service}/{*rest}", any(handlers::proxy))
        .with_state(state)
}
