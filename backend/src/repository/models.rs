//! Repository domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};

use crate::scope::Scope;

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
    pub deleted: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}
