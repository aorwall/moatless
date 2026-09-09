use async_trait::async_trait;
use moatless_types::access_control::Actor;
use moatless_types::repository::GitHostRepo;
use moatless_types::repository::RepositoryProvider;
use moatless_types::store::StoreError;

/// A git host whose credential is a single stored value, readable without
/// naming a repository.
///
/// GitHub and Forgejo are absent by construction. A GitHub installation token
/// is minted through the App, resolved from the repo's owner; a Forgejo
/// credential is stored per instance, so the repository names which one. Both
/// resolve through [`GitHostTokenIssuer::token`], which takes a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlainGitHost {
    Gitlab,
    Gitness,
}

impl PlainGitHost {
    pub fn provider(self) -> RepositoryProvider {
        match self {
            Self::Gitlab => RepositoryProvider::Gitlab,
            Self::Gitness => RepositoryProvider::Gitness,
        }
    }
}

/// Why an identity has no usable token for a repository.
///
/// [`Self::NoCredential`] is the only "not connected" answer. A dangling
/// `secret_ref`, an unknown App key or an unusable stored key is
/// [`Self::Broken`] — collapsing the two would send a caller down an anonymous
/// path instead of surfacing breakage.
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

    /// `repo` is boxed to keep the whole enum small: an `Actor` carries both a
    /// user id and a Workspace, and every fallible call through the port pays
    /// this variant's size in its `Result`.
    #[error("{actor}'s {} credential cannot reach {repo}", repo.provider())]
    Forbidden {
        actor: Actor,
        repo: Box<GitHostRepo>,
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

/// The one way to a git-host token.
///
/// Callers may rely on these facts:
///
/// - No scope walk is performed. Only two credentials are ever consulted: the
///   one the actor's Workspace pins for the host, and the actor's own. Another
///   identity's credential is never tried, at any scope.
/// - A Workspace pin outranks the identity's own credential, so every call made
///   on behalf of that Workspace presents the account it names. See
///   [`Actor::workspace_id`].
/// - The returned string is never a stored placeholder and never empty.
/// - Freshness is handled behind the port. Do not cache the returned string.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait GitHostTokenIssuer: Send + Sync {
    /// The token `actor` presents to `repo`'s host right now.
    async fn token(&self, actor: &Actor, repo: &GitHostRepo) -> Result<String, GitHostTokenError>;

    /// The token `actor` holds for a host that needs no repository to answer.
    async fn host_token(
        &self,
        actor: &Actor,
        host: PlainGitHost,
    ) -> Result<String, GitHostTokenError>;

    /// Whether `actor`'s GitHub credential is usable, with no repository in
    /// hand to name.
    ///
    /// For reporting connection state, not for calling GitHub: there is no
    /// token to return, because an App installation is resolved from a
    /// repository's owner and no repository is named here. Every path that
    /// actually reaches GitHub goes through [`Self::token`] instead.
    ///
    /// Answers for the credential the actor would really present — a Workspace
    /// pin, an App installation, a stored PAT — so a caller cannot conclude
    /// "not connected" from the absence of one particular kind of row.
    async fn github_is_usable(&self, actor: &Actor) -> Result<(), GitHostTokenError>;
}
