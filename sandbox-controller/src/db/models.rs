//! Database model structs matching the PostgreSQL schema.
//!
//! Only includes the fields needed for auto-provisioning.

/// Task row from the `tasks` table.
#[derive(sqlx::FromRow, Debug)]
pub struct TaskRow {
    pub task_id: String,
    pub repository_id: String,
    pub branch: Option<String>,
    pub created_by: Option<String>,
}

/// Lightweight provider token (access_token only) for credential resolution.
#[derive(sqlx::FromRow, Debug)]
pub struct ProviderAccessToken {
    pub access_token: String,
}

/// Repository row from the `repositories` table.
#[derive(sqlx::FromRow, Debug)]
pub struct RepositoryRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
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
}
