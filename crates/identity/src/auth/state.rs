//! Shared authentication dependencies and configured providers.

use super::{
    api_keys_dao::ApiKeyDao,
    cache::AuthCache,
    github::{registry::GitHubAppRegistry, service::GitHubAppService},
    oauth::provider::OAuthProvider,
    oidc::OidcService,
    pairing::DevicePairing,
    password::{dao::UserCredentialDao, throttle::LoginThrottle},
    provider_token_auth_dao::ProviderTokenAuthDao,
    provider_token_dao::ProviderTokenDao,
    sessions::SessionDao,
    users_dao::UserDao,
};
use crate::secret::service::SecretService;
use moatless_core::{config::Config, db::DbPool};
use moatless_types::auth::AuthProvider;
use std::sync::Arc;

/// Type-safe container for configured OAuth providers.
#[derive(Clone, Default)]
pub(super) struct OAuthProviders {
    pub github_app: Option<Arc<dyn OAuthProvider>>,
    pub auth0: Option<Arc<dyn OAuthProvider>>,
}

impl OAuthProviders {
    /// Look up a provider by `AuthProvider` enum.
    pub fn get(&self, provider: AuthProvider) -> Option<&Arc<dyn OAuthProvider>> {
        match provider {
            // Local password auth has no OAuth provider.
            AuthProvider::Local => None,
            AuthProvider::GitHubApp => self.github_app.as_ref(),
            AuthProvider::Auth0 => self.auth0.as_ref(),
            // Standards-compliant OIDC has its own typed client and does not
            // go through the provider-specific OAuth abstraction.
            AuthProvider::Oidc => None,
        }
    }
}

/// Shared state for auth routes.
#[derive(Clone)]
pub(super) struct AuthState {
    pub(super) db: DbPool,
    pub(super) config: Arc<Config>,
    pub(super) providers: Arc<OAuthProviders>,
    pub(super) oidc: Option<Arc<OidcService>>,
    pub(super) cache: AuthCache,
    pub(super) sessions: Arc<dyn SessionDao>,
    pub(super) api_keys: Arc<dyn ApiKeyDao>,
    /// Mints the codes that pair a client on another device to an account. The
    /// exchange half runs in the T3 compatibility module, against this same
    /// service.
    pub(super) pairing: Arc<DevicePairing>,
    pub(super) users: Arc<dyn UserDao>,
    /// Local password credentials (`user_credentials`).
    pub(super) credentials: Arc<dyn UserCredentialDao>,
    /// Shared per-email and per-source failed-login counters.
    pub(super) login_throttle: LoginThrottle,
    pub(super) provider_tokens: Arc<dyn ProviderTokenAuthDao>,
    /// DAO for credential lookups (full-row reads, refresh persistence).
    /// Separate from `provider_tokens` (which handles upsert/metadata paths).
    pub(super) provider_tokens_dao: Arc<dyn ProviderTokenDao>,
    /// Optional GitHub App service used to refresh expiring user tokens when
    /// reporting provider connection status on `/me`. Convenience handle to
    /// `github_apps.default_service()`.
    pub(super) github_app_service: Option<Arc<GitHubAppService>>,
    /// The one way to a git-host token. `/me` uses it to check whether the
    /// caller's own GitHub credential still resolves.
    pub(super) git_host_tokens: Arc<crate::git_host_tokens::GitHostTokens>,
    /// Registry of all configured GitHub Apps.
    pub(super) github_apps: Arc<GitHubAppRegistry>,
    /// One person's Forgejo credential, per instance. The connect flow writes
    /// through it; the settings routes read and administer the same rows.
    pub(super) forgejo_access: Arc<crate::forgejo::ForgejoAccess>,
    /// Encrypted secret store — holds the OAuth token stashed behind a PAT
    /// override, and the validated PAT itself.
    pub(super) secrets: Arc<SecretService>,
    /// Preview-URL access for the session gate — the one `may_access_task` bool
    /// the `ForwardAuth` gate needs (E-(b)), behind a port so this crate holds no
    /// `task` import.
    pub(super) task_access: Arc<dyn moatless_core::http_auth::TaskAccess>,
    /// Verifies the sandbox-issued `X-User-Token` on `exchange_token`, behind a
    /// port so this crate holds neither the token secret nor a `sandbox` import.
    pub(super) token_verifier: Arc<dyn moatless_core::http_auth::UserTokenVerifier>,
}
