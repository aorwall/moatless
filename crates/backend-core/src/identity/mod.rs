//! The seams the identity domain publishes to peer crates.
//!
//! Each port names what identity offers, returns values shaped for the caller,
//! and fails with [`StoreError`], so a peer needs neither the `users` /
//! `provider_tokens` rows nor the store driver behind them. `moatless-identity`
//! implements all five.

use std::collections::HashMap;

use async_trait::async_trait;
use moatless_types::agent::CodexScope;
use uuid::Uuid;

use crate::store::StoreError;

/// How a user is named to a human or to an external directory.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait UserDisplay: Send + Sync {
    /// Display name per user id. Ids with no user are absent from the map.
    async fn names(&self, user_ids: &[Uuid]) -> Result<HashMap<Uuid, String>, StoreError>;

    /// One user's display name, or `None` when no such user exists.
    async fn name(&self, user_id: &Uuid) -> Result<Option<String>, StoreError>;

    /// One user's email address, or `None` when the user has none or does not
    /// exist.
    async fn email(&self, user_id: &Uuid) -> Result<Option<String>, StoreError>;
}

/// Resolution of a user reference to the canonical internal id.
///
/// A reference is either the internal uuid or a legacy provider id preserved
/// from before the id migration; the caller does not have to know which it
/// holds.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait UserResolver: Send + Sync {
    async fn canonical_id(&self, reference: &str) -> Result<Option<Uuid>, StoreError>;
}

/// A user, as the log line that recorded a first sighting names them.
#[derive(Debug, Clone)]
pub struct LinkedUser {
    pub id: Uuid,
    pub login: String,
}

/// A user's identity at an external provider, and the link back to it.
///
/// An Adapter sees a sender by the provider's own id. The first sighting has no
/// link, so it falls back to the address the provider reports and records the
/// link it found; every later sighting resolves by id alone.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait ProviderIdentityLink: Send + Sync {
    /// The user linked to `provider_user_id` at `provider`.
    async fn user_at(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<LinkedUser>, StoreError>;

    /// The user who holds this address.
    async fn user_by_email(&self, email: &str) -> Result<Option<LinkedUser>, StoreError>;

    /// The user who connected `provider_user_id` at `provider` by authorizing
    /// an Adapter's app. Distinct from [`Self::user_at`]: that reads the
    /// directory link a person establishes, this reads the account behind a
    /// stored credential.
    async fn user_who_connected(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<String>, StoreError>;

    /// The user who connected under `login` at `provider` — the provider's own
    /// handle rather than its opaque id.
    async fn user_with_login(
        &self,
        provider: &str,
        login: &str,
    ) -> Result<Option<String>, StoreError>;

    /// Link `user_id` to `provider_user_id` at `provider`. Idempotent; a link
    /// already there was established deliberately and outranks a fresh
    /// sighting, so it is left alone. `true` when this call wrote the link.
    async fn link(
        &self,
        user_id: &Uuid,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<bool, StoreError>;
}

/// The git author identity a user's commits and pushes attribute to.
#[derive(Debug, Clone)]
pub struct GitIdentity {
    pub is_bot: bool,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
}

/// The facts about an actor that decide how it commits and what it may control.
///
/// Both methods take the actor id as a string and resolve it here. A string
/// that is not a user id is `Err`, not `Ok(None)`: an unresolvable reference
/// (the `system` sentinel, a `bot:` value) and a user that does not exist are
/// different facts, and a caller that wants to treat them alike must say so.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait CommitIdentity: Send + Sync {
    async fn git_identity(&self, user_id: &str) -> Result<Option<GitIdentity>, StoreError>;

    async fn is_platform_admin(&self, user_id: &str) -> Result<bool, StoreError>;
}

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
    async fn store_app_token(&self, provider: &str, token: &AppToken)
    -> Result<(), StoreError>;
}
