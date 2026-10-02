//! Artifact domain types.

use chrono::{DateTime, Utc};

/// A `task_artifacts` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Artifact {
    pub artifact_id: String,
    pub task_id: String,
    pub slug: String,
    pub title: String,
    pub reserved_version: i32,
    pub latest_version: i32,
    /// Every version whose upload and commit both succeeded.
    pub published_versions: Vec<i32>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A publish as the multipart route received it, before validation.
#[derive(Debug, Clone, Default)]
pub struct PublishRequest {
    pub title: String,
    pub slug: Option<String>,
    /// Filename of the `file` part, `None` when the form had no such part.
    pub file_name: Option<String>,
    pub html: Vec<u8>,
}

/// The id and version a publish holds before its objects are uploaded.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Reservation {
    pub artifact_id: String,
    pub reserved_version: i32,
}
