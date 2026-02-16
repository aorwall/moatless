//! Repository domain types — DB row structs.

use chrono::{DateTime, Utc};

// =============================================================================
// Repository (DB)
// =============================================================================

/// Repository row from the `repositories` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Repository {
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
    // CRUD fields
    pub description: Option<String>,
    pub provider: Option<String>,
    pub visibility: Option<String>,
    pub created_by: Option<String>,
    pub deleted: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}
