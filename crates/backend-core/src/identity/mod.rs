//! The seams the identity domain publishes to peer crates.
//!
//! Each port names what identity offers, returns values shaped for the caller,
//! and fails with [`StoreError`], so a peer needs neither the `users` /
//! `provider_tokens` rows nor the store driver behind them. `moatless-identity`
//! implements every one.

use std::collections::HashMap;

use async_trait::async_trait;
use moatless_types::access_control::{Actor, ScopeContext};
use moatless_types::agent::CodexScope;
use moatless_types::repository::{GitHostRepo, RepositoryProvider};
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

    /// The id a Moatless `login` is known by at `provider` — the reverse of
    /// [`Self::user_at`], reading the same link. What an outbound mention
    /// needs: the agent names a person in Moatless terms, and the Adapter can
    /// only address them by the provider's own id.
    async fn provider_id_for_login(
        &self,
        provider: &str,
        login: &str,
    ) -> Result<Option<String>, StoreError>;

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

/// The secrets one identity's runtime materializes.
///
/// Two channels, because they are delivered differently: env-style values
/// reach the agent as environment variables, ssh keys are written to `~/.ssh`
/// and never exposed as env. Each read walks `user > global` and marks the
/// records it used, so the audit trail is part of the read.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait SecretResolver: Send + Sync {
    /// Env-style secrets `user_id` can reach, keyed by secret name.
    async fn plaintext(&self, user_id: &str) -> Result<HashMap<String, String>, StoreError>;

    /// SSH private keys `user_id` can reach, keyed by secret name.
    async fn ssh_keys(&self, user_id: &str) -> Result<HashMap<String, String>, StoreError>;
}

/// Which secrets exist for a caller, by name alone.
///
/// Metadata only: no value is decrypted and none is returned. A caller that
/// needs a value reads it through [`SecretResolver`].
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait SecretCatalog: Send + Sync {
    /// Every secret key `ctx` may see — the global ones plus its own.
    async fn list(&self, ctx: &ScopeContext) -> Result<Vec<String>, StoreError>;
}

/// The Codex scopes a background refresh should sweep.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait CodexScopeReader: Send + Sync {
    /// Every scope holding a Codex credential with no permanent refresh
    /// failure recorded against it.
    async fn healthy_scopes(&self) -> Result<Vec<CodexScope>, StoreError>;
}

/// A git host whose credential is a single stored value, readable without
/// naming a repository.
///
/// GitHub is absent by construction: an installation token is minted through
/// the App, resolved from the repo's owner, so there is no stored GitHub token
/// and [`GitHostTokenIssuer::token`] is the only way to one.
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
    #[error("{actor} has not connected {provider}")]
    NoCredential {
        actor: Actor,
        provider: RepositoryProvider,
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
}
