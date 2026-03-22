//! Plugin repository domain types — DB row structs.

use chrono::{DateTime, Utc};

/// Row from the `plugin_repos` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct PluginRepo {
    pub id: String,
    pub name: String,
    pub git_url: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}
