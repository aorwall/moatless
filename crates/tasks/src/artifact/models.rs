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
    /// The published versions that were shown inline; their page is served
    /// with T3's bootstrap injected.
    pub inline_versions: Vec<i32>,
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
    /// Set by `inline=true`: show the page in the Task's thread.
    pub inline: Option<InlineRequest>,
}

/// How a publish asked to be shown inline, before validation.
#[derive(Debug, Clone, Default)]
pub struct InlineRequest {
    /// The frame height in CSS pixels; [`DEFAULT_INLINE_HEIGHT`] when absent.
    pub height: Option<i32>,
}

/// The frame height of an inline page that named none: room for most pages,
/// which the frame then shrinks to the height the page reports.
pub const DEFAULT_INLINE_HEIGHT: i32 = 1200;

/// The id and version a publish holds before its objects are uploaded.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Reservation {
    pub artifact_id: String,
    pub reserved_version: i32,
}
