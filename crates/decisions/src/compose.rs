//! Builds the decision routes from the backend config.

use std::sync::Arc;

use moatless_core::config::DecisionConfig;
use moatless_core::http_auth::UserTokenVerifier;
use moatless_types::http::HttpTimeouts;

use crate::provider::{Decider, DecisionProvider};
use crate::state::DecisionsState;

/// Upstream calls in flight at once, across every caller.
const MAX_IN_FLIGHT: usize = 16;

/// The decision routes and their OpenAPI metadata.
///
/// With no provider configured the route is still mounted and answers 503, so
/// a caller learns why rather than getting a 404.
pub fn decisions_router(
    config: &DecisionConfig,
    timeouts: HttpTimeouts,
    token_verifier: Arc<dyn UserTokenVerifier>,
) -> (axum::Router, utoipa::openapi::OpenApi) {
    let decider = config.provider.as_ref().map(|provider| {
        Arc::new(Decider::new(
            DecisionProvider::from_config(provider, timeouts),
            MAX_IN_FLIGHT,
        ))
    });
    if let Some(decider) = &decider {
        tracing::info!(
            provider = decider.provider.name(),
            model = decider.provider.model(),
            "Decision provider configured"
        );
    } else {
        tracing::info!("No decision provider configured; decisions answer 503");
    }
    crate::routes::decisions_router(DecisionsState {
        decider,
        token_verifier,
    })
}
