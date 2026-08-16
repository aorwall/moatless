//! Generic credential service: the pure persist + refresh half.
//!
//! Single point that:
//! - Refreshes a credential under a per-`(user_id, key)` mutex so parallel
//!   callers never trigger competing OAuth refreshes.
//! - Persists rotated values to `provider_tokens`.
//!
//! It does **not** reach up to running sandboxes or to open Tasks: the
//! in-flight guard and the write-through broadcast live one band up, in
//! the tasks-band `CredentialSync`, which wraps this service (9c).
//! So this module names neither `TaskDao` nor `SandboxConnector`.
//!
//! Callers of the wrapper (`CredentialSync`), for orientation:
//! - `sandbox::secrets` — resolves a fresh Codex blob for sandbox provisioning.
//! - `sandbox::grpc_server::update_credential` — sandbox pushed a rotated blob.
//! - `agent_harness_access::AgentHarnessAccess` — user saved a token via the UI.

pub mod codex;
pub mod error;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tracing::{debug, info};

use moatless_backend_core::scope::ScopeContext;
use moatless_core::db::DbPool;
use moatless_core::secret_keys::GITHUB_TOKEN;
use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
use moatless_identity::auth::provider_tokens::{
    AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER,
};
use moatless_identity::secret::models::SecretKind;
use moatless_identity::secret::schemas::PutSecretRequest;
use moatless_identity::secret::service::SecretService;

pub use self::error::CredentialError;

/// Where a [`CredentialService::store`] call originated. Used by [`broadcast`]
/// to skip the originating sandbox so a rotated value doesn't echo back as a
/// fresh `RefreshCredential` push.
#[derive(Debug, Clone)]
pub enum WriteSource {
    /// Sandbox-side rotation pushed via `BackendSandboxService::UpdateCredential`.
    SandboxRotation { task_id: String },
    /// Backend-driven refresh (e.g. GitHub App user-token expiry path).
    BackendRefresh,
    /// User clicked "save" / completed an OAuth flow in the settings UI.
    UserUiSave,
}

/// Which persist path [`CredentialService::store`] took. Returned so the
/// tasks-band `CredentialSync` can make the broadcast decision explicit at the
/// call site instead of hiding it inside the persist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOutcome {
    /// GitHub token routed to the encrypted secret store. Never broadcast —
    /// `should_broadcast("GITHUB_TOKEN")` is false.
    SecretRef,
    /// Dedup hit: the stored value already equalled `value`, nothing was
    /// persisted. Must not broadcast, or a no-op write echoes back.
    Unchanged,
    /// A new value was upserted. Broadcast iff `should_broadcast(key)`.
    Persisted,
}

/// Per-key broadcast policy. `pub(crate)` so the tasks-band `CredentialSync`
/// can gate its fan-out on it (the broadcast decision moved up with the
/// orchestration in 9c; the policy that keys are file-resident stays here).
pub fn should_broadcast(key: &str) -> bool {
    matches!(
        key,
        // File-resident credentials — sandboxes can't pull these on demand,
        // so the backend must push.
        "CODEX_AUTH_JSON" | "CLAUDE_CODE_OAUTH_TOKEN"
    )
}

/// Map a credential env-var key to the provider name in `provider_tokens`.
///
/// Returns `None` for keys that aren't backed by a provider row (e.g. raw
/// secrets that live elsewhere).
pub(crate) fn credential_key_to_provider(key: &str) -> Option<&'static str> {
    match key {
        "CODEX_AUTH_JSON" => Some("codex"),
        "OPENAI_API_KEY" => Some("openai"),
        "GITHUB_TOKEN" => Some("github"),
        "GITLAB_TOKEN" => Some("gitlab"),
        "GITNESS_TOKEN" => Some("gitness"),
        "CLAUDE_CODE_OAUTH_TOKEN" | "ANTHROPIC_API_KEY" => Some("claude"),
        _ => None,
    }
}

/// Generic credential gating point.
pub struct CredentialService {
    pub(in crate::credentials) provider_tokens: Arc<dyn ProviderTokenDao>,
    /// Encrypted secret store. Required to read or rotate a `secret_ref`
    /// GitHub token; `None` in tests / minimal wiring.
    secrets: Option<Arc<SecretService>>,
    /// Per-`(user_id, key)` mutex. The `()` payload is irrelevant — we lock
    /// purely for serialization. Auto-evicted after the TTL so inactive
    /// users' locks don't accumulate forever.
    locks: moka::future::Cache<(String, String), Arc<Mutex<()>>>,
    /// PostgreSQL pool for the Codex refresh path (advisory lock + CAS + raw
    /// `metadata` updates that don't fit the trait DAO).
    pub(in crate::credentials) db: Option<DbPool>,
    /// Shared HTTP client for the Codex refresh path. Reused so connection
    /// pools are warm across refreshes.
    pub(in crate::credentials) http: Option<Arc<reqwest::Client>>,
}

impl CredentialService {
    /// Build a service. This is the pure credential half — it persists and
    /// refreshes; the tasks-band `CredentialSync` wraps it to add the
    /// in-flight guard and the sandbox fan-out (9c).
    pub fn new(provider_tokens: Arc<dyn ProviderTokenDao>) -> Self {
        let locks = moka::future::Cache::builder()
            .max_capacity(10_000)
            .time_to_idle(Duration::from_mins(5))
            .build();

        Self {
            provider_tokens,
            secrets: None,
            locks,
            db: None,
            http: None,
        }
    }

    /// Wire the encrypted secret store used for `secret_ref` GitHub tokens.
    #[must_use]
    pub fn with_secrets(mut self, secrets: Arc<SecretService>) -> Self {
        self.secrets = Some(secrets);
        self
    }

    /// Wire in the Postgres pool + HTTP client used by the Codex refresh path.
    ///
    /// Call sites that don't need Codex refresh (e.g. unit tests) can leave
    /// these unset; `refresh_codex_locked` then falls back to a raw read.
    #[must_use]
    pub fn with_codex_refresh(mut self, db: DbPool, http: Arc<reqwest::Client>) -> Self {
        self.db = Some(db);
        self.http = Some(http);
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

    /// Persist a credential value. Returns which of the three persist paths
    /// ran, so the tasks-band `CredentialSync` can decide whether to fan the
    /// value out to running sandboxes (9c moved the broadcast up; the persist
    /// stays here).
    ///
    /// Per-`(user, key)` serialization prevents lost writes when two callers
    /// race (e.g. UI save + sandbox push at the same moment). Hash-based dedup
    /// short-circuits when the stored value is already up to date, which the
    /// caller uses to suppress an echo broadcast.
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
                &ScopeContext {
                    user_id: user_id.to_string(),
                    is_admin: false,
                },
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
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use moatless_identity::auth::codex_scope::CodexScope;
    use moatless_identity::auth::models::{
        GithubTokenPresence, ProviderAccessToken, ProviderToken,
    };
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// In-memory ProviderTokenDao for tests.
    struct InMemoryTokens {
        tokens: tokio::sync::Mutex<HashMap<(String, String), String>>,
        upsert_count: AtomicUsize,
    }

    impl InMemoryTokens {
        fn new() -> Self {
            Self {
                tokens: tokio::sync::Mutex::new(HashMap::new()),
                upsert_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ProviderTokenDao for InMemoryTokens {
        async fn find_provider_token(
            &self,
            user_id: &str,
            provider: &str,
        ) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
            let tokens = self.tokens.lock().await;
            Ok(tokens
                .get(&(user_id.to_string(), provider.to_string()))
                .map(|v| ProviderAccessToken {
                    access_token: v.clone(),
                }))
        }

        async fn find_provider_token_full(
            &self,
            _user_id: &str,
            _provider: &str,
        ) -> Result<Option<ProviderToken>, sqlx::Error> {
            Ok(None)
        }

        async fn upsert_token(
            &self,
            user_id: &str,
            provider: &str,
            access_token: &str,
            _refresh_token: Option<&str>,
            _token_type: Option<&str>,
            _metadata: Option<&serde_json::Value>,
        ) -> Result<(), sqlx::Error> {
            self.upsert_count.fetch_add(1, Ordering::SeqCst);
            self.tokens.lock().await.insert(
                (user_id.to_string(), provider.to_string()),
                access_token.to_string(),
            );
            Ok(())
        }

        async fn upsert_token_with_auth_method(
            &self,
            user_id: &str,
            provider: &str,
            _auth_method: &str,
            access_token: &str,
            refresh_token: Option<&str>,
            token_type: Option<&str>,
            metadata: Option<&serde_json::Value>,
        ) -> Result<(), sqlx::Error> {
            self.upsert_token(
                user_id,
                provider,
                access_token,
                refresh_token,
                token_type,
                metadata,
            )
            .await
        }

        async fn delete_token(&self, _user_id: &str, _provider: &str) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn find_codex_blob(
            &self,
            _scope: &CodexScope,
        ) -> Result<Option<ProviderToken>, sqlx::Error> {
            Ok(None)
        }

        async fn upsert_codex_blob(
            &self,
            _scope: &CodexScope,
            _access_token: &str,
            _refresh_token: Option<&str>,
            _token_type: Option<&str>,
            _metadata: Option<&serde_json::Value>,
        ) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn delete_codex_blob(&self, _scope: &CodexScope) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn find_github_row(
            &self,
            _user_id: &str,
        ) -> Result<Option<ProviderToken>, sqlx::Error> {
            Ok(None)
        }

        async fn upsert_github_row(
            &self,
            _user_id: &str,
            _auth_method: &str,
            _access_token: &str,
            _token_type: Option<&str>,
            _metadata: Option<&serde_json::Value>,
            _provider_user_id: Option<&str>,
            _provider_login: Option<&str>,
        ) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn delete_github_row(&self, _user_id: &str) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn find_github_scope_presence(
            &self,
            _user_id: &str,
        ) -> Result<Option<GithubTokenPresence>, sqlx::Error> {
            Ok(None)
        }
    }

    fn make_service(repo: Arc<dyn ProviderTokenDao>) -> CredentialService {
        CredentialService::new(repo)
    }

    #[tokio::test]
    async fn store_persists_value() {
        let repo = Arc::new(InMemoryTokens::new());
        let svc = make_service(repo.clone());
        let outcome = svc
            .store("u1", "CODEX_AUTH_JSON", "v1", WriteSource::UserUiSave)
            .await
            .unwrap();
        assert_eq!(outcome, StoreOutcome::Persisted);
        let got = repo
            .find_provider_token("u1", "codex")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.access_token, "v1");
    }

    #[tokio::test]
    async fn store_dedups_when_unchanged() {
        let repo = Arc::new(InMemoryTokens::new());
        let svc = make_service(repo.clone());
        svc.store("u1", "CODEX_AUTH_JSON", "v1", WriteSource::UserUiSave)
            .await
            .unwrap();
        let outcome = svc
            .store("u1", "CODEX_AUTH_JSON", "v1", WriteSource::UserUiSave)
            .await
            .unwrap();
        // First store does an upsert, second short-circuits before upsert.
        assert_eq!(outcome, StoreOutcome::Unchanged);
        assert_eq!(repo.upsert_count.load(Ordering::SeqCst), 1);
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
        let repo = Arc::new(InMemoryTokens::new());
        let svc = Arc::new(make_service(repo.clone()));

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
        assert!(repo.upsert_count.load(Ordering::SeqCst) >= 1);
    }
}
