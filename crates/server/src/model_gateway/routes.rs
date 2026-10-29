//! Where the model gateway mounts.

use axum::Router;
use axum::routing::post;

use super::anthropic;
use crate::state::AppState;

/// `/llm/anthropic/v1/messages` and its `count_tokens`, which a pod's Claude
/// reaches Anthropic through.
///
/// Mounted outside `/api`, and exempt from the auth middleware
/// (`moatless_identity::auth::middleware`): the route verifies the pod's task
/// token itself and resolves the model credential from the sandbox it names.
pub fn model_gateway_router(state: AppState) -> Router {
    Router::new()
        .route("/llm/anthropic/v1/messages", post(anthropic::messages))
        .route(
            "/llm/anthropic/v1/messages/count_tokens",
            post(anthropic::count_tokens),
        )
        .with_state(state)
}
