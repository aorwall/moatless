//! Tag domain types — DB row structs.

use chrono::{DateTime, Utc};

// =============================================================================
// Tag (DB)
// =============================================================================

/// Row from the `tags` table.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
pub struct Tag {
    pub id: i32,
    pub tag_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Join row from `task_tags` + `tags`.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskTag {
    pub task_id: String,
    pub tag_id: String,
    pub name: String,
}
