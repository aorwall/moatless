//! The state the team handlers extract from.

use std::sync::Arc;

use axum::extract::FromRef;

use super::dao::TeamDao;
use moatless_core::http_auth::UserTokenVerifier;

/// The two ports the team handlers reach: their own store, and the verifier
/// behind `AuthUser`.
#[derive(Clone)]
pub(crate) struct TeamState {
    pub(crate) teams: Arc<dyn TeamDao>,
    pub(crate) token_verifier: Arc<dyn UserTokenVerifier>,
}

impl FromRef<TeamState> for Arc<dyn TeamDao> {
    fn from_ref(state: &TeamState) -> Self {
        state.teams.clone()
    }
}

impl FromRef<TeamState> for Arc<dyn UserTokenVerifier> {
    fn from_ref(state: &TeamState) -> Self {
        state.token_verifier.clone()
    }
}
