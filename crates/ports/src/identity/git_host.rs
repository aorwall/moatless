use async_trait::async_trait;
use moatless_types::access_control::Actor;
use moatless_types::git_host::{GitHostAccount, RepositoryProvider};
use moatless_types::store::StoreError;

/// Why an identity has no usable token for a repository.
///
/// [`Self::NoCredential`] is the only "not connected" answer; a dangling
/// `secret_ref`, an unknown App key, or an unusable stored key is
/// [`Self::Broken`] instead, so breakage surfaces rather than looking
/// anonymous.
#[derive(Debug, thiserror::Error)]
pub enum GitHostTokenError {
    #[error("{actor} has not connected {provider}{}", host.as_deref().map_or(String::new(), |h| format!(" at {h}")))]
    NoCredential {
        actor: Actor,
        provider: RepositoryProvider,
        /// Which instance of a self-hostable `provider` is unconnected. `None`
        /// for a provider with one host, and for a credential resolved without
        /// naming a repository.
        host: Option<String>,
    },

    #[error("{actor}'s credential cannot access {account}")]
    Forbidden {
        actor: Actor,
        account: Box<GitHostAccount>,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    #[error("{actor}'s {provider} credential is unusable: {source}")]
    Broken {
        actor: Actor,
        provider: RepositoryProvider,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    #[error(transparent)]
    Store(#[from] StoreError),
}

/// A token that writes to one GitHub repository as the App its driver signed in
/// through, so the App, not the driver, authors what it posts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAuthoredToken {
    pub token: String,
    /// The driver's GitHub login. GitHub records no driver on what an App
    /// posts, so the caller credits this login in the text itself.
    pub driver_login: String,
}

/// The one way to a git-host token.
///
/// Only two credentials are ever tried, at any scope: the actor's Workspace pin
/// (which outranks the identity's own) and the actor's own; the returned string
/// is never a placeholder or empty, and must never be cached, since freshness
/// is handled behind the port.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait GitHostTokenIssuer: Send + Sync {
    /// The token `actor` presents for the selected account or instance.
    async fn token(
        &self,
        actor: &Actor,
        account: &GitHostAccount,
    ) -> Result<String, GitHostTokenError>;

    /// Whether `actor`'s GitHub credential is usable, with no repository in
    /// hand to name.
    ///
    /// For reporting connection state only — real calls go through
    /// [`Self::token`] — and answers for the credential `actor` would
    /// actually present, so one missing row kind can't read as "not connected".
    async fn github_is_usable(&self, actor: &Actor) -> Result<(), GitHostTokenError>;

    /// An installation token of the App `actor` connected GitHub through,
    /// scoped to `owner/repository` and capped by `actor`'s role there.
    ///
    /// `Ok(None)` means write with [`Self::token`] instead: a credential that
    /// is not an App sign-in, or a run-as acting for a person. A driver below
    /// write access, or an App not installed on `owner`, is `Forbidden`.
    async fn github_app_authored(
        &self,
        _actor: &Actor,
        _owner: &str,
        _repository: &str,
    ) -> Result<Option<AppAuthoredToken>, GitHostTokenError> {
        Ok(None)
    }
}
