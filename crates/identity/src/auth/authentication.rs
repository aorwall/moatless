//! Authentication state and the host-facing capsule.

use super::{
    api_keys, github, middleware, pairing, password, provider_token_dao, provider_tokens, routes,
    sessions, state, users,
};
use std::sync::Arc;


use crate::secret::service::SecretService;
use moatless_core::http_auth::{TaskAccess, UserTokenVerifier};

/// Application-owned ports needed by authentication. Construction pays this
/// wiring cost once; route callers see only [`Authentication`].
pub struct AuthenticationPorts {
    pub sessions: Arc<dyn sessions::SessionDao>,
    pub api_keys: Arc<dyn api_keys::ApiKeyDao>,
    pub pairing: Arc<pairing::DevicePairing>,
    pub users: Arc<dyn users::UserDao>,
    pub credentials: Arc<dyn password::dao::UserCredentialDao>,
    pub provider_tokens: Arc<dyn provider_tokens::ProviderTokenAuthDao>,
    pub provider_tokens_dao: Arc<dyn provider_token_dao::ProviderTokenDao>,
    pub github_app_service: Option<Arc<github::service::GitHubAppService>>,
    pub github_apps: Arc<github::registry::GitHubAppRegistry>,
    pub git_host_tokens: Arc<crate::git_host_tokens::GitHostTokens>,
    pub secrets: Arc<SecretService>,
    pub task_access: Arc<dyn TaskAccess>,
    pub token_verifier: Arc<dyn UserTokenVerifier>,
}

/// Deep authentication capsule: protocol routes and request authentication
/// share one private state graph and one session policy.
pub struct Authentication {
    pub(super) routes: state::AuthState,
    pub(super) layer: middleware::AuthMiddlewareState,
}

impl Authentication {
    pub fn routes(&self) -> (axum::Router, utoipa::openapi::OpenApi) {
        routes::auth_router(self.routes.clone())
    }

    pub fn layer(&self) -> middleware::AuthMiddlewareState {
        self.layer.clone()
    }
}
