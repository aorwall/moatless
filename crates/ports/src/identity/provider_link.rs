use async_trait::async_trait;
use moatless_types::store::StoreError;
use uuid::Uuid;

/// A user, as the log line that recorded a first sighting names them.
#[derive(Debug, Clone)]
pub struct LinkedUser {
    pub id: Uuid,
    pub login: String,
}

/// A user's identity at an external provider, and the link back to it.
///
/// An Adapter sees a sender by the provider's own id; a first sighting has no
/// link, so it falls back to the reported address and records what it finds,
/// and every later sighting resolves by id alone.
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

    /// The id a Moatless `login` is known by at `provider` — the reverse of
    /// [`Self::user_at`].
    ///
    /// What an outbound mention needs: the agent names a person in Moatless
    /// terms, and the Adapter can only address them by the provider's own id.
    async fn provider_id_for_login(
        &self,
        provider: &str,
        login: &str,
    ) -> Result<Option<String>, StoreError>;

    /// The user who connected `provider_user_id` at `provider` by authorizing
    /// an Adapter's app.
    ///
    /// Distinct from [`Self::user_at`]: that reads the directory link a person
    /// establishes, this reads the account behind a stored credential.
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
