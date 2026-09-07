use async_trait::async_trait;
use moatless_types::store::StoreError;

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
