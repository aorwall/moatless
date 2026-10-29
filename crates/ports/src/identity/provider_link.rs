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
/// A sender's first sighting has no link, so the Adapter falls back to the
/// reported address and records it; later sightings resolve by provider id.
/// Each default answers as a directory with no links, for test doubles.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait ProviderIdentityLink: Send + Sync {
    /// The user linked to `provider_user_id` at `provider`.
    async fn user_at(
        &self,
        _provider: &str,
        _provider_user_id: &str,
    ) -> Result<Option<LinkedUser>, StoreError> {
        Ok(None)
    }

    /// The user who holds this address.
    async fn user_by_email(&self, _email: &str) -> Result<Option<LinkedUser>, StoreError> {
        Ok(None)
    }

    /// The id a Moatless `login` is known by at `provider` — the reverse of
    /// [`Self::user_at`].
    ///
    /// What an outbound mention needs: the agent names a person in Moatless
    /// terms, and the Adapter can only address them by the provider's own id.
    async fn provider_id_for_login(
        &self,
        _provider: &str,
        _login: &str,
    ) -> Result<Option<String>, StoreError> {
        Ok(None)
    }

    /// The id `user_id` is known by at `provider`.
    async fn provider_id_for_user(
        &self,
        _provider: &str,
        _user_id: &Uuid,
    ) -> Result<Option<String>, StoreError> {
        Ok(None)
    }

    /// The user who connected `provider_user_id` at `provider` by authorizing
    /// an Adapter's app.
    ///
    /// Distinct from [`Self::user_at`]: that reads the directory link a person
    /// establishes, this reads the account behind a stored credential.
    async fn user_who_connected(
        &self,
        _provider: &str,
        _provider_user_id: &str,
    ) -> Result<Option<String>, StoreError> {
        Ok(None)
    }

    /// The user who connected under `login` at `provider` — the provider's own
    /// handle rather than its opaque id.
    async fn user_with_login(
        &self,
        _provider: &str,
        _login: &str,
    ) -> Result<Option<String>, StoreError> {
        Ok(None)
    }

    /// Link `user_id` to `provider_user_id` at `provider`. Idempotent; a link
    /// already there was established deliberately and outranks a fresh
    /// sighting, so it is left alone. `true` when this call wrote the link.
    async fn link(
        &self,
        _user_id: &Uuid,
        _provider: &str,
        _provider_user_id: &str,
    ) -> Result<bool, StoreError> {
        Ok(false)
    }
}
