use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::auth::models::GithubTokenPresence;
use crate::auth::provider_token_dao::ProviderTokenDao;

use super::CredentialError;

/// The one way to read, persist and refresh a provider-token credential.
pub struct CredentialService {
    pub(in crate::credentials) provider_tokens: Arc<dyn ProviderTokenDao>,
    /// Per-`(user_id, key)` mutex. The `()` payload is irrelevant — we lock
    /// purely for serialization. Auto-evicted after the TTL so inactive users'
    /// locks don't accumulate forever.
    locks: moka::future::Cache<(String, String), Arc<Mutex<()>>>,
    /// Shared HTTP client for the Codex refresh path. Reused so connection
    /// pools are warm across refreshes.
    pub(in crate::credentials) http: Option<Arc<reqwest::Client>>,
    /// The endpoint `http` posts a Codex refresh to; set with it.
    pub(in crate::credentials) codex_refresh_url: Option<String>,
}

impl CredentialService {
    /// Build a service. This is the pure credential half; the tasks-band
    /// `CredentialSync` walks its scopes.
    pub(crate) fn new(provider_tokens: Arc<dyn ProviderTokenDao>) -> Self {
        let locks = moka::future::Cache::builder()
            .max_capacity(10_000)
            .time_to_idle(Duration::from_mins(5))
            .build();

        Self {
            provider_tokens,
            locks,
            http: None,
            codex_refresh_url: None,
        }
    }

    /// Wire in the HTTP client the Codex refresh path calls `refresh_url` with.
    ///
    /// Left unset (e.g. unit tests), `refresh_codex_locked` falls back to a raw
    /// read.
    #[must_use]
    pub(crate) fn with_codex_refresh(
        mut self,
        http: Arc<reqwest::Client>,
        refresh_url: String,
    ) -> Self {
        self.http = Some(http);
        self.codex_refresh_url = Some(refresh_url);
        self
    }

    /// Acquire the per-`(user, key)` mutex.
    pub(in crate::credentials) async fn acquire_lock(
        &self,
        user_id: &str,
        key: &str,
    ) -> Arc<Mutex<()>> {
        let key = (user_id.to_string(), key.to_string());
        self.locks
            .get_with(key, async { Arc::new(Mutex::new(())) })
            .await
    }

    /// What the caller's GitHub row says about itself: its `auth_method`, the
    /// `login` it was minted for, and whether a refresh gave up for good.
    pub async fn github_presence(
        &self,
        user_id: &str,
    ) -> Result<Option<GithubTokenPresence>, CredentialError> {
        Ok(self
            .provider_tokens
            .find_github_scope_presence(user_id)
            .await?)
    }
}
