use super::handlers::{health, livez};
use crate::state::SandboxState;
use axum::{Router, routing::get};

pub(crate) fn router(state: SandboxState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/livez", get(livez))
        .with_state(state)
}
