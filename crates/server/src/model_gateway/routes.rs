//! Where the model gateway mounts.

use axum::Router;
use axum::routing::{get, post};

use super::{anthropic, openai};
use crate::state::AppState;

/// The `/llm/{provider}` endpoints a pod's Claude and Codex call.
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
        .route("/llm/openai/v1/responses", post(openai::responses))
        .route("/llm/openai/v1/responses/compact", post(openai::compact))
        .route("/llm/openai/v1/models", get(openai::models))
        .with_state(state)
}
