//! Construct authentication dependencies once at host startup.

use super::oauth::provider::OAuthProvider;
use super::state::OAuthProviders;
use super::{
    Authentication, AuthenticationPorts, cache, error, middleware, oidc, password, sessions, state,
};
use moatless_core::{config::Config, db::DbPool};
use moatless_types::auth::AuthProvider;
use std::sync::Arc;

impl Authentication {
    /// Build authentication with owner-private session persistence.
    /// ```compile_fail
    /// use moatless_identity::auth::sessions::dao::PgSessionDao;
    /// ```
    pub fn build(
        config: Arc<Config>,
        db: DbPool,
        ports: AuthenticationPorts,
    ) -> Result<Self, error::AuthError> {
        let sessions: Arc<dyn sessions::SessionDao> = Arc::new(sessions::PgSessionDao(db.clone()));
        let providers = OAuthProviders::from_config(&config);
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
            sessions: sessions.clone(),
            token_verifier: ports.token_verifier.clone(),
            cache: cache.clone(),
            config: config.clone(),
        };
        let routes = state::AuthState {
            db,
            config,
            providers: Arc::new(providers),
            oidc,
            cache,
            sessions,
            api_keys: ports.api_keys,
            pairing: ports.pairing,
            users: ports.users,
            credentials: ports.credentials,
            login_throttle,
            provider_tokens: ports.provider_tokens,
            provider_tokens_dao: ports.provider_tokens_dao,
            github_app_service: ports.github_app_service,
            github_apps: ports.github_apps,
            git_host_tokens: ports.git_host_tokens,
            secrets: ports.secrets,
            task_access: ports.task_access,
            token_verifier: ports.token_verifier,
        };
        Ok(Self { routes, layer })
    }
}

impl OAuthProviders {
    /// Build providers from application config.
    ///
    /// Each provider is only created when its required env vars are set.
    pub(super) fn from_config(config: &Config) -> Self {
        let github_app =
            super::oauth::github_app::GitHubAppOAuthProvider::from_config(config).map(|p| {
                tracing::info!("GitHub App OAuth provider configured");
                Arc::new(p) as Arc<dyn OAuthProvider>
            });
        if config.auth.provider == AuthProvider::GitHubApp && github_app.is_none() {
            tracing::warn!(
                "AUTH_PROVIDER=github_app but GitHubAppOAuthProvider failed to initialize \
                 — check GITHUB_APP_CLIENT_ID, GITHUB_APP_CLIENT_SECRET, GITHUB_APP_CALLBACK_URL"
            );
        }
        let auth0 = super::oauth::auth0::Auth0OAuthProvider::from_config(config).map(|p| {
            tracing::info!("Auth0 OAuth provider configured");
            Arc::new(p) as Arc<dyn OAuthProvider>
        });
        Self { github_app, auth0 }
    }
}
