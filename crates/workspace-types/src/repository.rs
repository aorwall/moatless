//! Repository: a connection to a git repo, and the host it lives on.
//!
//! Run configuration belongs to the Workspace, not here. A Task references a
//! Workspace, never a Repository directly.

use chrono::{DateTime, Utc};

use moatless_types::access_control::Scope;

crate::string_enum!(
    /// Kind of repository source.
    RepositoryKind {
        Git => "git",
        Remote => "remote",
        Template => "template",
    }
);

use moatless_types::git_host::GitHostAccount;
pub use moatless_types::git_host::RepositoryProvider;

/// A connection to a git repo — its name, remote URL, default branch, and host.
#[derive(Debug, Clone)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub kind: RepositoryKind,
    pub remote: Option<RepositoryRemote>,
    pub default_branch: Option<String>,
    pub description: Option<String>,
    /// The git host this repository lives on. `None` when it was never set or
    /// the persisted value is one this build does not know.
    pub provider: Option<RepositoryProvider>,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    pub deleted: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Remote facts resolved by the workspace domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRemote {
    pub url: String,
    pub host_repo: Option<GitHostRepo>,
}

/// The one host GitHub serves, which no other provider here is fixed to.
pub const GITHUB_HOST: &str = "github.com";

/// A repository identity on a git host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHostRepo {
    provider: RepositoryProvider,
    owner: String,
    name: String,
    host: String,
}

impl GitHostRepo {
    /// A git-host identity whose fields the caller already holds.
    pub fn new(
        provider: RepositoryProvider,
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    /// A GitHub repository whose owner and name a caller already holds — a
    /// webhook subject, say, where there is no remote URL to parse.
    pub fn github(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            provider: RepositoryProvider::Github,
            owner: owner.into(),
            name: name.into(),
            host: GITHUB_HOST.to_string(),
        }
    }

    /// The account or instance used to resolve this repository's credential.
    pub fn credential_account(&self) -> GitHostAccount {
        match self.provider {
            RepositoryProvider::Github => GitHostAccount::Github {
                owner: self.owner.clone(),
                repository: Some(self.name.clone()),
            },
            RepositoryProvider::Forgejo => GitHostAccount::Forgejo {
                host: self.host.clone(),
            },
            RepositoryProvider::Gitlab => GitHostAccount::Gitlab,
        }
    }

    pub fn provider(&self) -> RepositoryProvider {
        self.provider
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The host to address this repository on. Always [`GITHUB_HOST`] for
    /// GitHub, and whatever the remote URL named for a self-hostable provider.
    pub fn host(&self) -> &str {
        &self.host
    }
}

impl std::fmt::Display for GitHostRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}
