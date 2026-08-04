//! Generic credential service: serialized refresh + write-through broadcast.
//!
//! Single point that:
//! - Reads-or-refreshes a credential under a per-`(user_id, key)` mutex so
//!   parallel callers never trigger competing OAuth refreshes.
//! - Persists rotated values to `provider_tokens`.
//! - Broadcasts rotations to every running sandbox the user owns so file-resident
//!   credentials (Codex `auth.json`, Claude `.credentials`) stay live.
//!
//! Callers:
//! - `sandbox::secrets::resolve_task_credentials` — gets fresh GitHub App tokens
//!   for sandbox provisioning.
//! - `sandbox::grpc_server::update_credential` — sandbox pushed a rotated blob.
//! - `provider_settings::handlers::*` — user saved a token via the UI.
//! - `auth::token_refresh::ensure_fresh_token` — internal use, now wrapped here.

pub mod codex;
pub mod error;
pub mod github;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::{
    ProviderTokenAuthDao, AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER,
};
use crate::auth::token_refresh::ensure_fresh_token_with_dao;
use crate::db::DbPool;
use moatless_backend_core::sandbox::SandboxConnector;
use crate::sandbox::secrets::GITHUB_TOKEN;
use crate::secret::models::SecretKind;
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::SecretService;
use crate::task::dao::TaskDao;
use moatless_backend_core::scope::{Scope, ScopeContext};

pub use self::codex::CodexScope;
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

/// Per-key broadcast policy.
fn should_broadcast(key: &str) -> bool {
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

/// Bundle of dependencies needed to call out to running sandboxes.
///
/// Kept as a separate struct so unit tests can construct a `CredentialService`
/// without spinning up real K8s/Docker plumbing.
pub struct BroadcastContext {
    pub connector: Arc<dyn SandboxConnector>,
    pub tasks: Arc<dyn TaskDao>,
}

/// Generic credential gating point.
pub struct CredentialService {
    pub(in crate::credentials) provider_tokens: Arc<dyn ProviderTokenDao>,
    provider_token_auth: Option<Arc<dyn ProviderTokenAuthDao>>,
    github_app_service: Option<Arc<GitHubAppService>>,
    /// Encrypted secret store. Required to read or rotate a `secret_ref`
    /// GitHub token; `None` in tests / minimal wiring.
    secrets: Option<Arc<SecretService>>,
    /// Per-`(user_id, key)` mutex. The `()` payload is irrelevant — we lock
    /// purely for serialization. Auto-evicted after the TTL so inactive
    /// users' locks don't accumulate forever.
    locks: moka::future::Cache<(String, String), Arc<Mutex<()>>>,
    pub(in crate::credentials) broadcast_ctx: Option<BroadcastContext>,
    /// PostgreSQL pool for the Codex refresh path (advisory lock + CAS + raw
    /// `metadata` updates that don't fit the trait DAO).
    pub(in crate::credentials) db: Option<DbPool>,
    /// Shared HTTP client for the Codex refresh path. Reused so connection
    /// pools are warm across refreshes.
    pub(in crate::credentials) http: Option<Arc<reqwest::Client>>,
}

impl CredentialService {
    /// Build a service. `broadcast_ctx` is optional — when omitted, [`store`]
    /// still persists but skips broadcasting (used in tests and by the gRPC
    /// server which constructs its own state without a sandbox provider).
    pub fn new(
        provider_tokens: Arc<dyn ProviderTokenDao>,
        provider_token_auth: Option<Arc<dyn ProviderTokenAuthDao>>,
        github_app_service: Option<Arc<GitHubAppService>>,
        broadcast_ctx: Option<BroadcastContext>,
    ) -> Self {
        let locks = moka::future::Cache::builder()
            .max_capacity(10_000)
            .time_to_idle(Duration::from_mins(5))
            .build();

        Self {
            provider_tokens,
            provider_token_auth,
            github_app_service,
            secrets: None,
            locks,
            broadcast_ctx,
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
    /// these unset; `ensure_fresh_codex` then falls back to a raw read.
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

    /// Read-or-refresh-then-store, serialized per `(user_id, key)`.
    ///
    /// Today only supports GitHub App user-tokens — other credentials are
    /// returned as-is from `provider_tokens`. The serialization makes it safe
    /// to add Codex / Claude refresh paths later without revisiting the
    /// race-condition story.
    pub async fn ensure_fresh(&self, user_id: &str, key: &str) -> Result<String, CredentialError> {
        // Codex refresh has its own dedicated entry point that takes a
        // per-row mutex AND a Postgres advisory lock; it must not run under
        // the generic `(user, key)` mutex. Default to user scope here for
        // back-compat with the generic API; callers that want team / global
        // resolution use the explicit `ensure_fresh_codex(scope, …)` entry
        // point on the resolver.
        if key == "CODEX_AUTH_JSON" {
            return self
                .ensure_fresh_codex(CodexScope::user(user_id), None)
                .await;
        }

        let lock = self.acquire_lock(user_id, key).await;
        let _guard = lock.lock().await;

        match key {
            "GITHUB_TOKEN" => {
                let token_auth = self.provider_token_auth.as_ref();
                let github_app = self.github_app_service.as_ref();
                if let (Some(token_auth), Some(github_app)) = (token_auth, github_app) {
                    let token = ensure_fresh_token_with_dao(
                        self.provider_tokens.as_ref(),
                        token_auth.as_ref(),
                        github_app,
                        user_id,
                    )
                    .await?;
                    Ok(token)
                } else {
                    // GitHub App not configured — fall through to raw read.
                    self.read_raw(user_id, key).await
                }
            }
            _ => self.read_raw(user_id, key).await,
        }
    }

    /// Read the encrypted PAT behind a `secret_ref` GitHub row, if that's what
    /// this user has. `Ok(None)` means the row isn't a reference row.
    async fn read_github_secret_ref(
        &self,
        user_id: &str,
    ) -> Result<Option<String>, CredentialError> {
        let Some(row) = self
            .provider_tokens
            .find_provider_token_full(user_id, "github")
            .await?
        else {
            return Ok(None);
        };
        if row.auth_method.as_deref() != Some(AUTH_METHOD_SECRET_REF) {
            return Ok(None);
        }
        let secrets = self.secrets.as_ref().ok_or_else(|| {
            CredentialError::Internal(format!(
                "User '{user_id}' has a secret_ref GitHub token but no secret service is wired"
            ))
        })?;
        secrets
            .resolve_user_secret(user_id, GITHUB_TOKEN, Some(SecretKind::ProviderToken))
            .await?
            .map(Some)
            .ok_or_else(|| {
                CredentialError::NotFound(format!(
                    "GITHUB_TOKEN secret missing for secret_ref user {user_id}"
                ))
            })
    }

    async fn read_raw(&self, user_id: &str, key: &str) -> Result<String, CredentialError> {
        let provider = credential_key_to_provider(key)
            .ok_or_else(|| CredentialError::Internal(format!("Unknown credential key: {key}")))?;

        // A secret_ref row's `access_token` column holds only a sentinel.
        if key == GITHUB_TOKEN {
            if let Some(token) = self.read_github_secret_ref(user_id).await? {
                return Ok(token);
            }
        }

        let row = self
            .provider_tokens
            .find_provider_token(user_id, provider)
            .await?
            .ok_or_else(|| {
                CredentialError::NotFound(format!(
                    "No {provider} provider_token for user {user_id}"
                ))
            })?;
        Ok(row.access_token)
    }

    /// Persist a credential value and broadcast to running sandboxes.
    ///
    /// - Per-`(user, key)` serialization prevents lost writes when two callers
    ///   race (e.g. UI save + sandbox push at the same moment).
    /// - Hash-based dedup short-circuits the broadcast if the stored value is
    ///   already up to date — prevents echo loops where a broadcast write
    ///   propagates back through the originating sandbox's watcher.
    /// - The originating sandbox (if any) is excluded from the broadcast so
    ///   it doesn't immediately re-receive its own rotation.
    pub async fn store(
        &self,
        user_id: &str,
        key: &str,
        value: &str,
        source: WriteSource,
    ) -> Result<(), CredentialError> {
        let provider = credential_key_to_provider(key)
            .ok_or_else(|| CredentialError::Internal(format!("Unknown credential key: {key}")))?;

        let lock = self.acquire_lock(user_id, key).await;
        let _guard = lock.lock().await;

        // A GitHub token must never land in `provider_tokens.access_token` in
        // the clear — route it to the encrypted store and keep the row as a
        // reference. Without this a sandbox-side rotation would resurrect a
        // plaintext row.
        if key == GITHUB_TOKEN && self.secrets.is_some() {
            self.store_github_secret_ref(user_id, value).await?;
            info!(
                user_id,
                provider,
                ?source,
                "Persisted GitHub token as secret"
            );
            return Ok(());
        }

        // Dedup: skip persistence + broadcast when the value is unchanged.
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
            return Ok(());
        }

        self.provider_tokens
            .upsert_token(user_id, provider, value, None, None, None)
            .await?;

        info!(user_id, provider, ?source, "Persisted credential");

        if should_broadcast(key) {
            let except_task_id = match &source {
                WriteSource::SandboxRotation { task_id } => Some(task_id.as_str()),
                _ => None,
            };
            // `store` is user-scoped (per-user provider_tokens upsert), so
            // fan out to that user's running sandboxes. Team / global
            // rotations go through `broadcast_to_scope` directly from
            // `ensure_fresh_codex`.
            self.broadcast_to_scope(&CodexScope::user(user_id), except_task_id, key, value)
                .await;
        }

        Ok(())
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
                    scope: Some(Scope::User),
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

    /// Push a freshly stored credential to every running sandbox whose
    /// effective user resolves through `scope`. Fire-and-forget per sandbox
    /// — failures are logged.
    ///
    /// v1 fan-out is coarse: for team / global scopes we push to every open
    /// codex task within the scope, including users who have their own row
    /// and don't need the push. Harmless because the sandbox watcher applies
    /// the value only when it differs from the in-memory blob.
    pub(in crate::credentials) async fn broadcast_to_scope(
        &self,
        scope: &CodexScope,
        except_task_id: Option<&str>,
        key: &str,
        value: &str,
    ) {
        let Some(ref ctx) = self.broadcast_ctx else {
            return;
        };

        let scope_kind = scope.scope_kind().to_string();
        let scope_id = match scope {
            CodexScope::User(id) => id.clone(),
            CodexScope::Global => String::new(),
        };
        let except = except_task_id.unwrap_or("").to_string();
        let task_ids = match ctx
            .tasks
            .list_open_codex_tasks_for_scope(scope_kind, scope_id, except)
            .await
        {
            Ok(ids) => ids,
            Err(e) => {
                warn!(
                    error = %e,
                    scope = %scope.display(),
                    "broadcast_to_scope: failed to list codex tasks"
                );
                return;
            }
        };

        for task_id in task_ids {
            let connector = Arc::clone(&ctx.connector);
            let key = key.to_string();
            let value = value.to_string();
            tokio::spawn(async move {
                let client = match connector.sandbox_client(&task_id).await {
                    Ok(c) => c,
                    Err(e) => {
                        debug!(task_id, error = %e, "broadcast: client connect failed");
                        return;
                    }
                };
                if let Err(e) = client.refresh_credential(&task_id, &key, &value).await {
                    debug!(task_id, error = %e, "broadcast: refresh_credential gRPC failed");
                } else {
                    info!(task_id, key, "broadcast: refresh_credential delivered");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::models::{GithubTokenPresence, ProviderAccessToken, ProviderToken};
    use crate::credentials::github::scope::GitHubScope;
    use async_trait::async_trait;
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
            _scope: &GitHubScope,
        ) -> Result<Option<ProviderToken>, sqlx::Error> {
            Ok(None)
        }

        async fn upsert_github_row(
            &self,
            _scope: &GitHubScope,
            _auth_method: &str,
            _access_token: &str,
            _token_type: Option<&str>,
            _metadata: Option<&serde_json::Value>,
            _provider_user_id: Option<&str>,
            _provider_login: Option<&str>,
        ) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn delete_github_row(&self, _scope: &GitHubScope) -> Result<(), sqlx::Error> {
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
        CredentialService::new(repo, None, None, None)
    }

    #[tokio::test]
    async fn store_persists_value() {
        let repo = Arc::new(InMemoryTokens::new());
        let svc = make_service(repo.clone());
        svc.store("u1", "CODEX_AUTH_JSON", "v1", WriteSource::UserUiSave)
            .await
            .unwrap();
        let got = svc.read_raw("u1", "CODEX_AUTH_JSON").await.unwrap();
        assert_eq!(got, "v1");
    }

    #[tokio::test]
    async fn store_dedups_when_unchanged() {
        let repo = Arc::new(InMemoryTokens::new());
        let svc = make_service(repo.clone());
        svc.store("u1", "CODEX_AUTH_JSON", "v1", WriteSource::UserUiSave)
            .await
            .unwrap();
        svc.store("u1", "CODEX_AUTH_JSON", "v1", WriteSource::UserUiSave)
            .await
            .unwrap();
        // First store does an upsert, second short-circuits before upsert.
        assert_eq!(repo.upsert_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn read_raw_returns_stored_value() {
        let repo = Arc::new(InMemoryTokens::new());
        let svc = make_service(repo.clone());
        repo.upsert_token("u1", "codex", "v1", None, None, None)
            .await
            .unwrap();
        let got = svc.read_raw("u1", "CODEX_AUTH_JSON").await.unwrap();
        assert_eq!(got, "v1");
    }

    #[tokio::test]
    async fn read_raw_returns_not_found_for_missing_token() {
        let repo = Arc::new(InMemoryTokens::new());
        let svc = make_service(repo.clone());
        let err = svc.read_raw("u1", "CODEX_AUTH_JSON").await.unwrap_err();
        assert!(matches!(err, CredentialError::NotFound(_)));
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
