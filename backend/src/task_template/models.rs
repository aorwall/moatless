//! Task template domain types — DB row structs.

use chrono::{DateTime, Utc};

use crate::scope::Scope;

// =============================================================================
// TaskTemplate (DB)
// =============================================================================

/// Task template row from the `task_templates` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskTemplate {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub repository_id: String,
    pub agent_type: String,
    pub agent_mode: Option<String>,
    pub default_model: Option<String>,
    pub initial_prompt: Option<String>,
    pub skills: Option<serde_json::Value>,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub featured: bool,
    pub display_order: i32,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
