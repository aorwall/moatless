use super::{Installation, handlers};
use axum::{Router, routing::get};
use std::sync::Arc;

/// Browser-only workspace installation endpoints.
pub fn router(installation: Option<Arc<Installation>>) -> Router {
    Router::new()
        .route("/api/v1/auth/linear/install", get(handlers::install))
        .route("/api/v1/auth/linear/callback", get(handlers::callback))
        .with_state(installation)
}
