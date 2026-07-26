//! Plugin domain types — DB row structs.

use chrono::{DateTime, Utc};

/// Row from the `plugins` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub git_url: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}
