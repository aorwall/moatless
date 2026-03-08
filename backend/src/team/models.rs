//! Team domain types — DB row structs.

use chrono::{DateTime, Utc};

use crate::shared_models::Settings;

/// Row from the `teams` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub external_id: Option<String>,
    pub external_provider: Option<String>,
    pub is_default: bool,
    pub settings: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Team {
    /// Deserialize the JSONB `settings` column into the typed [`Settings`] struct.
    pub fn parsed_settings(&self) -> Settings {
        serde_json::from_value(self.settings.clone()).unwrap_or_default()
    }
}

/// Row from the `team_members` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TeamMember {
    pub team_id: String,
    pub user_id: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}
