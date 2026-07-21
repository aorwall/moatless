//! Repository domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};

use moatless_backend_core::scope::Scope;

pub use moatless_api_schemas::enums::{RepositoryKind, RepositoryProvider};
pub use moatless_api_schemas::repository::ResourceConfig;

// =============================================================================
// Repository (DB)
// =============================================================================

/// Repository row from the `repositories` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
    /// Opt-in Docker-in-Docker access (privileged `dind` sidecar) for this repo's
    /// sandboxes — enables Testcontainers. Defaults to `None`/false.
    pub docker_access: Option<bool>,
    /// Per-repo Kubernetes ServiceAccount for this repo's sandbox pods. When set,
    /// overrides the global default (`K8S_SANDBOX_SERVICE_ACCOUNT`). Used for
    /// "shadow" dev sandboxes that need the backend's RBAC. Defaults to `None`.
    pub sandbox_service_account: Option<String>,
    /// Per-repo network profile. When set, sandbox pods carry the label
    /// `moatless.io/network-profile=<value>` for a static (GitOps-owned)
    /// NetworkPolicy to select. Selects a policy only; grants nothing. Used for
    /// sealed benchmark runs. Defaults to `None` (no profile label).
    pub network_profile: Option<String>,
    pub system_prompt: Option<String>,
    pub setup_commands: Option<serde_json::Value>,
    pub env_vars: Option<serde_json::Value>,
    pub servers: Option<serde_json::Value>,
    pub plugins: Option<serde_json::Value>,
    // Preview config (flattened columns)
    pub server_start_commands: Option<serde_json::Value>,
    pub startup_timeout_seconds: Option<i32>,
    pub build_command: Option<String>,
    pub build_output_directory: Option<String>,
    pub enable_inspector: Option<bool>,
    pub auto_reload: Option<bool>,
    // CRUD fields
    pub description: Option<String>,
    pub provider: Option<RepositoryProvider>,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    pub resources: Option<serde_json::Value>,
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
