//! Construct authentication dependencies once at host startup.

use super::api_keys_dao::{ApiKeyDao, PgApiKeyDao};
use super::oauth::provider::OAuthProvider;
use super::pairing::DevicePairing;
use super::pairing_dao::{PgPairingApiKeyIssuer, PgPairingCodeDao};
use super::password::dao::{PgUserCredentialDao, UserCredentialDao};
use super::provider_token_auth_dao::{PgProviderTokenAuthDao, ProviderTokenAuthDao};
use super::provider_token_dao::{PgProviderTokenDao, ProviderTokenDao};
use super::state::OAuthProviders;
use super::users_dao::{PgUserDao, UserDao};
use super::{
    Authentication, AuthenticationPorts, cache, error, middleware, oidc, password, sessions, state,
};
use moatless_core::{config::Config, db::DbPool};
use moatless_types::auth::AuthProvider;
use std::sync::Arc;

impl Authentication {
    /// Build authentication with owner-private session persistence.
    pub fn build(
        config: Arc<Config>,
        db: DbPool,
        ports: AuthenticationPorts,
    ) -> Result<Self, error::AuthError> {
        let sessions: Arc<dyn sessions::SessionDao> = Arc::new(sessions::PgSessionDao(db.clone()));
        let api_keys: Arc<dyn ApiKeyDao> = Arc::new(PgApiKeyDao(db.clone()));
        let users: Arc<dyn UserDao> = Arc::new(PgUserDao(db.clone()));
        let credentials: Arc<dyn UserCredentialDao> = Arc::new(PgUserCredentialDao(db.clone()));
        let provider_tokens: Arc<dyn ProviderTokenAuthDao> =
            Arc::new(PgProviderTokenAuthDao(db.clone()));
        let provider_tokens_dao: Arc<dyn ProviderTokenDao> =
            Arc::new(PgProviderTokenDao(db.clone()));
        let pairing = Arc::new(DevicePairing::new(
            Arc::new(PgPairingCodeDao(db.clone())),
            Arc::new(PgPairingApiKeyIssuer(api_keys.clone())),
        ));
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
            api_keys: api_keys.clone(),
            users: users.clone(),
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
            api_keys,
            pairing,
            users,
            credentials,
            login_throttle,
            provider_tokens,
            provider_tokens_dao,
            github_app_service: ports.github_app_service,
            github_apps: ports.github_apps,
            git_host_tokens: ports.git_host_tokens,
            forgejo_access: ports.forgejo_access,
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
