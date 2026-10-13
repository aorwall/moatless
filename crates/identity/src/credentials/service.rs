use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tracing::{debug, info};

use moatless_core::secret_keys::GITHUB_TOKEN;
use moatless_types::access_control::ScopeContext;
use moatless_types::secret::SecretKind;

use crate::auth::models::GithubTokenPresence;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::{AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER};
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::SecretService;

use super::models::credential_key_to_provider;
#[cfg(test)]
use super::should_broadcast;
use super::{CredentialError, StoreOutcome, WriteSource};

/// The one way to read, persist and refresh a provider-token credential.
pub struct CredentialService {
    pub(in crate::credentials) provider_tokens: Arc<dyn ProviderTokenDao>,
    /// Encrypted secret store. Required to read or rotate a `secret_ref` GitHub
    /// token; `None` in tests / minimal wiring.
    secrets: Option<Arc<SecretService>>,
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
    /// Build a service. This is the pure credential half — it persists and
    /// refreshes; the tasks-band `CredentialSync` wraps it to add the in-flight
    /// guard and the sandbox fan-out.
    pub(crate) fn new(provider_tokens: Arc<dyn ProviderTokenDao>) -> Self {
        let locks = moka::future::Cache::builder()
            .max_capacity(10_000)
            .time_to_idle(Duration::from_mins(5))
            .build();

        Self {
            provider_tokens,
            secrets: None,
            locks,
            http: None,
            codex_refresh_url: None,
        }
    }

    /// Wire the encrypted secret store used for `secret_ref` GitHub tokens.
    #[must_use]
    pub(crate) fn with_secrets(mut self, secrets: Arc<SecretService>) -> Self {
        self.secrets = Some(secrets);
        self
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

    /// Persist a credential value; returns which of three persist paths ran,
    /// for `CredentialSync` to decide fan-out.
    ///
    /// Per-`(user, key)` serialization prevents lost writes on a race, and
    /// dedup short-circuits an unchanged value so the caller can suppress an
    /// echo broadcast.
    pub async fn store(
        &self,
        user_id: &str,
        key: &str,
        value: &str,
        source: WriteSource,
    ) -> Result<StoreOutcome, CredentialError> {
        let provider = credential_key_to_provider(key)
            .ok_or_else(|| CredentialError::Internal(format!("Unknown credential key: {key}")))?;

        let lock = self.acquire_lock(user_id, key).await;
        let _guard = lock.lock().await;

        // Path 1 (SecretRef): a GitHub token must never land in
        // `provider_tokens.access_token` in the clear — route it to the
        // encrypted store and keep the row as a reference. Never broadcast:
        // `should_broadcast("GITHUB_TOKEN")` is false and the sentinel row
        // carries no fan-out-able value.
        if key == GITHUB_TOKEN && self.secrets.is_some() {
            self.store_github_secret_ref(user_id, value).await?;
            info!(
                user_id,
                provider,
                ?source,
                "Persisted GitHub token as secret"
            );
            return Ok(StoreOutcome::SecretRef);
        }

        // Path 2 (Unchanged): dedup — skip persistence when the value is
        // unchanged. The caller must not broadcast, or a no-op write echoes
        // through the originating sandbox's watcher.
        let existing = self
            .provider_tokens
            .find_provider_token(user_id, provider)
            .await?;
        if existing
            .as_ref()
            .is_some_and(|row| row.access_token == value)
        {
            debug!(
                user_id,
                provider, "CredentialService::store skipped — value unchanged"
            );
            return Ok(StoreOutcome::Unchanged);
        }

        // Path 3 (Persisted): a new value was upserted. The caller broadcasts
        // iff `should_broadcast(key)`.
        self.provider_tokens
            .upsert_token(user_id, provider, value, None, None, None)
            .await?;

        info!(user_id, provider, ?source, "Persisted credential");

        Ok(StoreOutcome::Persisted)
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

    /// Write a GitHub token to the encrypted store and point the
    /// `provider_tokens` row at it.
    ///
    /// Existing `metadata` and `token_type` are carried over so the row keeps
    /// serving the config endpoint's `login` and PAT-override detection.
    async fn store_github_secret_ref(
        &self,
        user_id: &str,
        value: &str,
    ) -> Result<(), CredentialError> {
        let secrets = self
            .secrets
            .as_ref()
            .ok_or_else(|| CredentialError::Internal("secret service not wired".to_string()))?;

        let existing = self
            .provider_tokens
            .find_provider_token_full(user_id, "github")
            .await?;

        secrets
            .put_secret(
                &ScopeContext::user(user_id.to_string()),
                user_id,
                PutSecretRequest {
                    scope: Some(moatless_api_schemas::enums::Scope::User),
                    key: GITHUB_TOKEN.to_string(),
                    kind: SecretKind::ProviderToken,
                    provider: Some("github".to_string()),
                    value: Some(value.to_string()),
                    external_ref: None,
                },
            )
            .await?;

        let token_type = existing.as_ref().and_then(|row| row.token_type.clone());
        let metadata = existing.as_ref().and_then(|row| row.metadata.clone());
        self.provider_tokens
            .upsert_token_with_auth_method(
                user_id,
                "github",
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
                None,
                token_type.as_deref(),
                metadata.as_ref(),
                None,
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::test_support::InMemoryTokens;

    #[tokio::test]
    async fn store_persists_value() {
        let repo = InMemoryTokens::empty();
        let svc = CredentialService::new(repo.clone());
        let outcome = svc
            .store("u1", "CODEX_AUTH_JSON", "v1", WriteSource::UserUiSave)
            .await
            .unwrap();
        assert_eq!(outcome, StoreOutcome::Persisted);
        assert_eq!(repo.token("u1", "codex").await.as_deref(), Some("v1"));
    }

    #[tokio::test]
    async fn store_dedups_when_unchanged() {
        let repo = InMemoryTokens::empty();
        let svc = CredentialService::new(repo.clone());
        svc.store("u1", "CODEX_AUTH_JSON", "v1", WriteSource::UserUiSave)
            .await
            .unwrap();
        let outcome = svc
            .store("u1", "CODEX_AUTH_JSON", "v1", WriteSource::UserUiSave)
            .await
            .unwrap();
        // First store does an upsert, second short-circuits before upsert.
        assert_eq!(outcome, StoreOutcome::Unchanged);
        assert_eq!(repo.upsert_count(), 1);
    }

    #[test]
    fn should_broadcast_only_for_file_resident_keys() {
        // File-resident creds: backend must push, agents can't pull.
        assert!(should_broadcast("CODEX_AUTH_JSON"));
        assert!(should_broadcast("CLAUDE_CODE_OAUTH_TOKEN"));
        // Helper-pull creds: no push needed, broker pulls fresh on each call.
        assert!(!should_broadcast("GITHUB_TOKEN"));
        assert!(!should_broadcast("OPENAI_API_KEY"));
    }

    #[test]
    fn credential_key_to_provider_maps_known_keys() {
        assert_eq!(credential_key_to_provider("CODEX_AUTH_JSON"), Some("codex"));
        assert_eq!(credential_key_to_provider("GITHUB_TOKEN"), Some("github"));
        assert_eq!(
            credential_key_to_provider("CLAUDE_CODE_OAUTH_TOKEN"),
            Some("claude")
        );
        assert_eq!(credential_key_to_provider("OPENAI_API_KEY"), Some("openai"));
        assert_eq!(credential_key_to_provider("NOT_A_REAL_KEY"), None);
    }

    #[tokio::test]
    async fn parallel_stores_serialize_per_user_key() {
        // Two parallel stores for the same (user, key) should both complete
        // (the mutex serializes them) and end with one of the values stored.
        let repo = InMemoryTokens::empty();
        let svc = Arc::new(CredentialService::new(repo.clone()));

        let svc1 = svc.clone();
        let svc2 = svc.clone();

        let h1 = tokio::spawn(async move {
            svc1.store("u1", "CODEX_AUTH_JSON", "value-A", WriteSource::UserUiSave)
                .await
        });
        let h2 = tokio::spawn(async move {
            svc2.store("u1", "CODEX_AUTH_JSON", "value-B", WriteSource::UserUiSave)
                .await
        });

        h1.await.unwrap().unwrap();
        h2.await.unwrap().unwrap();
        // Both writes should land — ordering is non-deterministic but at least
        // one upsert per distinct value succeeded.
        assert!(repo.upsert_count() >= 1);
    }
}
