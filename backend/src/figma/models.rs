//! Figma domain types — DB row structs.

use chrono::{DateTime, Utc};

// =============================================================================
// FigmaImport (DB)
// =============================================================================

/// Row from the `figma_imports` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct FigmaImport {
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
