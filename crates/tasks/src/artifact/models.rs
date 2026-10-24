//! Artifact domain types.

use chrono::{DateTime, Utc};
use moatless_api_schemas::artifact::RecordWriter;
use moatless_types::task::Visibility;

use super::rules::RecordRules;

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
    /// See [`DataAccess::accepts_data`]; the latest publish's choice.
    pub accepts_data: bool,
    /// See [`DataAccess::notify_on_data`]; the latest publish's choice.
    pub notify_on_data: bool,
    /// See [`DataAccess::rules`]; the latest publish's choice.
    #[sqlx(json(nullable))]
    pub record_rules: Option<RecordRules>,
    /// `Public` lets any signed-in user read the pages and records, whatever
    /// the Task's visibility; the latest publish's choice.
    #[sqlx(try_from = "String")]
    pub visibility: Visibility,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A publish as the multipart route received it, before validation.
#[derive(Debug, Clone)]
pub struct PublishRequest {
    pub title: String,
    pub slug: Option<String>,
    /// Filename of the `file` part, `None` when the form had no such part.
    pub file_name: Option<String>,
    pub html: Vec<u8>,
    /// Set by `inline=true`: show the page in the Task's thread.
    pub inline: Option<InlineRequest>,
    pub data: DataAccess,
    /// See [`Artifact::visibility`]; `Public` unless the form says otherwise.
    pub visibility: Visibility,
}

impl Default for PublishRequest {
    fn default() -> Self {
        Self {
            title: String::new(),
            slug: None,
            file_name: None,
            html: Vec::new(),
            inline: None,
            data: DataAccess::default(),
            visibility: Visibility::Public,
        }
    }
}

/// What a publish lets its page do with Artifact records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataAccess {
    /// The page is served with a `moatless.artifact` script that saves and
    /// lists records as the reader viewing it.
    pub accepts_data: bool,
    /// Each record a reader saves also queues a message to the Task's agent.
    /// Refused together with `rules`, whose `notify` lists replace it.
    pub notify_on_data: bool,
    /// The operations a page may perform, per key prefix; `None` allows all.
    pub rules: Option<RecordRules>,
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

/// An `artifact_records` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ArtifactRecord {
    pub artifact_id: String,
    pub key: String,
    pub data: serde_json::Value,
    pub submitted_by: String,
    #[sqlx(try_from = "String")]
    pub writer: RecordWriter,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
