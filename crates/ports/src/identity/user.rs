use async_trait::async_trait;
use moatless_types::store::StoreError;
use std::collections::HashMap;
use uuid::Uuid;

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
/// from before the id migration; the caller need not know which it holds.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait UserResolver: Send + Sync {
    async fn canonical_id(&self, reference: &str) -> Result<Option<Uuid>, StoreError>;

    /// Resolve a run-as reference — a uuid or a login — to the identity a
    /// run-as check reads. `None` when nobody matches.
    ///
    /// Accepts a login because a run-as target is named by a person, who knows
    /// `soap-reviewer[bot]` and not its uuid.
    async fn run_as_target(&self, reference: &str) -> Result<Option<RunAsUser>, StoreError>;
}

/// A user a run-as request may name.
#[derive(Debug, Clone)]
pub struct RunAsUser {
    pub id: Uuid,
    pub login: String,
    /// `true` for a GitHub App bot identity — the only account an admin may
    /// borrow. See [`crate::identity::UserResolver::run_as_target`].
    pub is_bot: bool,
}
