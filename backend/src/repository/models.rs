//! Repository domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};

use moatless_backend_core::scope::Scope;

pub use moatless_api_schemas::enums::{RepositoryKind, RepositoryProvider};
pub use moatless_api_schemas::repository::ResourceConfig;

// =============================================================================
// Repository (DB)
// =============================================================================

/// Repository row from the `repositories` table.
///
/// A repository is only a connection to a git repo. Run-configuration lives on
/// the workspace; the physical run-config columns still exist on this table but
/// are no longer read or written here (`SELECT *` + `FromRow` ignore them).
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub description: Option<String>,
    pub provider: Option<RepositoryProvider>,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    /// Provider reachability of `remote_url`, stamped when the repo is
    /// auto-created by a git-synced workspace: `unchecked` | `ok` | `not_found`
    /// | `forbidden` | `error`. Defaults to `unchecked`.
    pub access_status: String,
    /// When `access_status` was last stamped.
    pub access_checked_at: Option<DateTime<Utc>>,
    /// Human-readable detail for a non-`ok` `access_status`.
    pub access_error: Option<String>,
    pub deleted: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}
