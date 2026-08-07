//! Repository domain types — typed enums and DB row structs.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use moatless_backend_core::scope::Scope;

pub use moatless_api_schemas::enums::{RepositoryKind, RepositoryProvider};
pub use moatless_api_schemas::repository::ResourceConfig;

/// Per-server run configuration. Stored on the workspace's `servers` array and
/// consumed by the sandbox providers to build containers/pods.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub start_commands: Option<Vec<String>>,
    #[serde(default)]
    pub startup_timeout_seconds: Option<u32>,
    #[serde(default)]
    pub auto_reload: bool,
    #[serde(default)]
    pub enable_inspector: bool,
    #[serde(default)]
    pub default: bool,
    /// Per-server Docker image. Falls back to sandbox_image if not set.
    #[serde(default)]
    pub docker_image: Option<String>,
    /// Per-server environment variables.
    #[serde(default)]
    pub env_vars: Option<HashMap<String, String>>,
    /// Per-server resource requests/limits. Falls back to the repo-wide
    /// `resources` when unset. Only `requests`/`limits` apply; storage/cache
    /// fields are repo-wide and ignored here.
    #[serde(default)]
    pub resources: Option<ResourceConfig>,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_config_minimal_json_deserializes_with_defaults() {
        let json = r#"{"name":"files","port":4000}"#;
        let config: ServerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "files");
        assert_eq!(config.port, 4000);
        assert!(!config.default);
        assert!(!config.enable_inspector);
        assert!(config.docker_image.is_none());
        assert!(config.start_commands.is_none());
        assert!(config.env_vars.is_none());
        assert!(config.label.is_none());
    }
}
