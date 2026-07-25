//! File storage domain types — DB row structs.

use chrono::{DateTime, Utc};

use moatless_backend_core::scope::Scope;

// =============================================================================
// FileMetadata (DB)
// =============================================================================

/// Row from the `file_metadata` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct FileMetadata {
    pub file_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    pub uploaded_at: DateTime<Utc>,
}
