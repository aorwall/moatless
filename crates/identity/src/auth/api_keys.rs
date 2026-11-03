//! API key rows.

use chrono::{DateTime, Utc};

#[derive(sqlx::FromRow, Debug)]
pub struct ApiKey {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub scopes: Option<String>,
    pub group_id: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    /// When set, each use moves `expires_at` to this many seconds past it.
    pub idle_ttl_secs: Option<i64>,
    pub created_at: DateTime<Utc>,
}
