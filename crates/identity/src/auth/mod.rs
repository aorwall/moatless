//! Authentication and user management module.
//!
//! Handles login flows (local email/password, GitHub App, Auth0, OIDC),
//! session management, API key management, and Traefik ForwardAuth verification.

pub mod api_key;
pub mod api_keys;
pub mod cache;
pub mod codex_auth_json;
pub mod cookie;
pub mod credentials;
pub mod error;
pub mod github;
pub mod jwt;
pub mod middleware;
pub mod models;
pub mod oauth;
pub mod oidc;
pub mod pairing;
pub mod password;
pub mod provider_secret_adapter;
pub mod provider_token_dao;
pub mod provider_tokens;
pub mod routes;
pub mod schemas;
pub mod secrets;
pub mod session;
pub mod sessions;
// `test` stays first so the census keeps scoring this fake as test-context; the
// feature exposes it to other crates' tests (see Cargo.toml `test-support`).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod test_user;
pub mod token_refresh;
pub mod users;

use std::sync::Arc;

use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_core::config::Config;
use moatless_core::db::DbPool;

use moatless_core::http_auth::{TaskAccess, UserTokenVerifier};
use crate::secret::service::SecretService;

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
    pub adapter_connections: Arc<dyn AdapterConnectionDao>,
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
    routes: routes::AuthState,
    layer: middleware::AuthMiddlewareState,
}

impl Authentication {
    pub fn build(
        config: Arc<Config>,
        db: DbPool,
        ports: AuthenticationPorts,
    ) -> Result<Self, error::AuthError> {
        let providers = routes::OAuthProviders::from_config(&config);
        let oidc = config
            .auth
            .oidc
            .clone()
            .map(|oidc| oidc::OidcService::new(oidc, db.clone()).map(Arc::new))
            .transpose()?;
        let cache = cache::AuthCache::new(config.auth.cache_ttl_secs);
        let login_throttle = password::throttle::LoginThrottle::new(db.clone());
        let layer = middleware::AuthMiddlewareState {
            api_keys: ports.api_keys.clone(),
            users: ports.users.clone(),
            sessions: ports.sessions.clone(),
            token_verifier: ports.token_verifier.clone(),
            cache: cache.clone(),
            config: config.clone(),
        };
        let routes = routes::AuthState {
            db,
            config,
            providers: Arc::new(providers),
            oidc,
            cache,
            sessions: ports.sessions,
            api_keys: ports.api_keys,
            pairing: ports.pairing,
            users: ports.users,
            credentials: ports.credentials,
            login_throttle,
            provider_tokens: ports.provider_tokens,
            provider_tokens_dao: ports.provider_tokens_dao,
            adapter_connections: ports.adapter_connections,
            github_app_service: ports.github_app_service,
            github_apps: ports.github_apps,
            git_host_tokens: ports.git_host_tokens,
            secrets: ports.secrets,
            task_access: ports.task_access,
            token_verifier: ports.token_verifier,
        };
        Ok(Self { routes, layer })
    }

    pub fn routes(&self) -> (axum::Router, utoipa::openapi::OpenApi) {
        routes::auth_router(self.routes.clone())
    }

    pub fn layer(&self) -> middleware::AuthMiddlewareState {
        self.layer.clone()
    }
}
