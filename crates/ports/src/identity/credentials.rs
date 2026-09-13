use async_trait::async_trait;
use moatless_types::agent::CodexScope;
use moatless_types::store::StoreError;

/// The stored credentials an agent harness runs with. Each method yields the
/// access token alone — no refresh material, no metadata.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait HarnessCredentials: Send + Sync {
    /// The codex auth blob stored at `scope`.
    async fn codex_blob(&self, scope: &CodexScope) -> Result<Option<String>, StoreError>;

    /// `user_id`'s access token for `provider`.
    async fn provider_token(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<String>, StoreError>;
}

/// The deployment-wide OAuth credential an Adapter authenticates its own app
/// with — one row per provider, belonging to no user and no team.
#[derive(Debug, Clone)]
pub struct AppToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub metadata: Option<serde_json::Value>,
}

/// The app-level OAuth credential an Adapter reads and rotates.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait AppCredentials: Send + Sync {
    async fn app_token(&self, provider: &str) -> Result<Option<AppToken>, StoreError>;

    /// Persist a freshly-exchanged credential. Idempotent on `provider`.
    async fn store_app_token(&self, provider: &str, token: &AppToken) -> Result<(), StoreError>;
}

/// The Codex scopes a background refresh should sweep.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait CodexScopeReader: Send + Sync {
    /// Every scope holding a Codex credential with no permanent refresh failure
    /// recorded against it.
    async fn healthy_scopes(&self) -> Result<Vec<CodexScope>, StoreError>;
}
