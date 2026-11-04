//! Authentication state and the host-facing capsule.

use super::{github, middleware, routes, state};
use std::sync::Arc;

use crate::secret::service::SecretService;
use moatless_core::http_auth::{SandboxReach, TaskAccess, UserTokenVerifier};

/// Application-owned ports needed by authentication. Construction pays this
/// wiring cost once; route callers see only [`Authentication`].
pub struct AuthenticationPorts {
    pub github_app_service: Option<Arc<github::service::GitHubAppService>>,
    pub github_apps: Arc<github::registry::GitHubAppRegistry>,
    pub git_host_tokens: Arc<crate::git_host_tokens::GitHostTokens>,
    pub forgejo_access: Arc<crate::forgejo::ForgejoAccess>,
    pub secrets: Arc<SecretService>,
    pub task_access: Arc<dyn TaskAccess>,
    pub token_verifier: Arc<dyn UserTokenVerifier>,
    pub sandbox_reach: Arc<dyn SandboxReach>,
}

/// Deep authentication capsule: protocol routes and request authentication
/// share one private state graph and one session policy.
/// ```compile_fail
/// use moatless_identity::auth::sessions;
/// ```
pub struct Authentication {
    pub(super) routes: state::AuthState,
    pub(super) layer: middleware::AuthMiddlewareState,
}

impl Authentication {
    /// Mount the authentication routes without exposing their persistence.
    pub fn routes(&self) -> (axum::Router, utoipa::openapi::OpenApi) {
        routes::auth_router(self.routes.clone())
    }

    /// Clone the middleware state without exposing its session store.
    /// ```compile_fail
    /// use moatless_identity::auth::Authentication;
    /// fn f(a: &Authentication) { let _ = a.layer().sessions; }
    /// ```
    pub fn layer(&self) -> middleware::AuthMiddlewareState {
        self.layer.clone()
    }
}
