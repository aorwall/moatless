//! Database model structs matching the PostgreSQL schema.

use chrono::{DateTime, Utc};

// =========================================================================
// Tasks
// =========================================================================

/// Full task row from the `tasks` table.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
pub struct TaskRow {
    pub task_id: String,
    pub short_id: Option<String>,
    pub created_at: String,
    pub last_message_at: String,
    pub agent_type: String,
    pub repository_id: String,
    pub status: String,
    pub total_cost_usd: f64,
    pub site_url: Option<String>,
    pub template_id: Option<String>,
    pub vibe: Option<bool>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub site_id: Option<String>,
    pub site_provider: Option<String>,
    pub branch: Option<String>,
    pub created_by: Option<String>,
    pub visibility: String,
    pub created_at_db: Option<DateTime<Utc>>,
}

// =========================================================================
// Repositories
// =========================================================================

/// Repository row from the `repositories` table.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
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
    // CRUD fields
    pub description: Option<String>,
    pub provider: Option<String>,
    pub visibility: Option<String>,
    pub created_by: Option<String>,
    pub deleted: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

// =========================================================================
// Provider tokens
// =========================================================================

/// Lightweight provider token (access_token only) for credential resolution.
#[derive(sqlx::FromRow, Debug)]
pub struct ProviderAccessToken {
    pub access_token: String,
}

// =========================================================================
// Task messages
// =========================================================================

/// Row from the `task_messages` table.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
pub struct TaskMessageRow {
    pub id: i32,
    pub task_id: String,
    pub uuid: String,
    pub message_type: String,
    pub user_type: String,
    pub parent_uuid: Option<String>,
    pub is_sidechain: Option<bool>,
    pub cwd: Option<String>,
    pub session_id: Option<String>,
    pub turn_number: Option<i32>,
    pub version: Option<String>,
    pub git_branch: Option<String>,
    pub request_id: Option<String>,
    pub timestamp: String,
    pub message_role: String,
    pub message_content: serde_json::Value,
    pub assistant_message_id: Option<String>,
    pub assistant_message_type: Option<String>,
    pub assistant_message_model: Option<String>,
    pub assistant_stop_reason: Option<String>,
    pub assistant_stop_sequence: Option<String>,
    pub assistant_usage_input_tokens: Option<i32>,
    pub assistant_usage_output_tokens: Option<i32>,
    pub assistant_usage_cache_creation_input_tokens: Option<i32>,
    pub assistant_usage_cache_read_input_tokens: Option<i32>,
    pub tool_use_result: Option<serde_json::Value>,
    pub attachments: Option<serde_json::Value>,
    pub agent_session_id: Option<String>,
    pub turn_status: Option<String>,
    pub turn_result: Option<String>,
    pub will_not_resume: Option<bool>,
    pub run_metrics: Option<serde_json::Value>,
    pub agent_mode: Option<String>,
    pub model: Option<String>,
    pub resume_session_id: Option<String>,
    pub sent_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
}

// =========================================================================
// Task message contexts
// =========================================================================

/// Row from the `task_message_contexts` table.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
pub struct TaskMessageContextRow {
    pub id: i32,
    pub task_message_id: i32,
    #[sqlx(rename = "type")]
    pub context_type: String,
    pub reference_id: String,
    pub data: serde_json::Value,
    pub created_at: Option<DateTime<Utc>>,
}

// =========================================================================
// Task message content blocks
// =========================================================================

/// Row from the `task_message_content_blocks` table.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
pub struct TaskMessageContentBlockRow {
    pub id: i32,
    pub task_message_id: i32,
    pub sort_order: i32,
    pub block_type: String,
    pub show_in_ui: bool,
    pub text_content: Option<String>,
    pub tool_use_id: Option<String>,
    pub tool_result_content: Option<String>,
    pub tool_result_is_error: Option<bool>,
    pub image_media_type: Option<String>,
    pub image_data: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

// =========================================================================
// Tags
// =========================================================================

/// Row from the `tags` table.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
pub struct TagRow {
    pub id: i32,
    pub tag_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Join row from `task_tags` + `tags`.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskTagRow {
    pub task_id: String,
    pub tag_id: String,
    pub name: String,
}

// =========================================================================
// Commands
// =========================================================================

/// Row from the `commands` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct CommandRow {
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub conditions: Option<serde_json::Value>,
    pub icon: Option<String>,
    pub variant: Option<String>,
    pub active: Option<bool>,
    pub will_not_resume: Option<bool>,
    pub agent_mode: Option<String>,
}

// =========================================================================
// File metadata
// =========================================================================

/// Row from the `file_metadata` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct FileMetadataRow {
    pub file_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub uploaded_at: DateTime<Utc>,
}

// =========================================================================
// Users (display info)
// =========================================================================

/// Lightweight user row for display name resolution.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
pub struct UserDisplayRow {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
}

// =========================================================================
// Figma imports
// =========================================================================

/// Row from the `figma_imports` table.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
pub struct FigmaImportRow {
    pub import_id: String,
    pub file_id: Option<String>,
    pub file_key: Option<String>,
    pub node_id: Option<String>,
    pub file_name: Option<String>,
    pub node_name: Option<String>,
    pub node_metadata: Option<serde_json::Value>,
    pub has_image: Option<bool>,
    pub include_image_in_prompt: Option<bool>,
    pub created_by: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

// =========================================================================
// Provider tokens (full row)
// =========================================================================

/// Full provider token row from the `provider_tokens` table.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
pub struct ProviderTokenRow {
    pub id: i32,
    pub user_id: String,
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}
