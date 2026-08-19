//! Repository: a connection to a git repo, and the host it lives on.
//!
//! Run configuration belongs to the Workspace, not here. A Task references a
//! Workspace, never a Repository directly.

use chrono::{DateTime, Utc};

use crate::access_control::Scope;

crate::string_enum!(
    /// Kind of repository source.
    RepositoryKind {
        Git => "git",
        Remote => "remote",
        Template => "template",
    }
);

crate::string_enum!(
    /// Git hosting provider.
    RepositoryProvider {
        Github => "github",
        Gitlab => "gitlab",
        Gitness => "gitness",
    }
);

/// A connection to a git repo — its name, remote URL, default branch, and host.
#[derive(Debug, Clone)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
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
