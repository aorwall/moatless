//! Router state for the decision routes.

use std::sync::Arc;

use axum::extract::FromRef;
use moatless_core::http_auth::UserTokenVerifier;

use crate::provider::Decider;

#[derive(Clone)]
pub(crate) struct DecisionsState {
    /// `None` when no provider is configured; the route then answers 503.
    pub(crate) decider: Option<Arc<Decider>>,
    pub(crate) token_verifier: Arc<dyn UserTokenVerifier>,
}

impl FromRef<DecisionsState> for Arc<dyn UserTokenVerifier> {
    fn from_ref(state: &DecisionsState) -> Self {
        state.token_verifier.clone()
    }
}
