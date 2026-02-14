//! Command domain types — DB row structs.

// =============================================================================
// Command (DB)
// =============================================================================

/// Row from the `commands` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Command {
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
