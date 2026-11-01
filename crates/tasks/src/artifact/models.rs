//! Artifact domain types.

use chrono::{DateTime, Utc};
use moatless_types::memory::MemoryScope;
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
    /// See [`DataAccess::collections`]; the latest publish's choice.
    #[sqlx(json)]
    pub collections: Vec<AttachedCollection>,
    /// `Public` lets any signed-in user read the pages and records, whatever
    /// the Task's visibility; the latest publish or `set_visibility` that
    /// named one decides.
    #[sqlx(try_from = "String")]
    pub visibility: Visibility,
    /// The Artifact this one was forked from, with [`Self::forked_from_version`];
    /// may name an Artifact deleted since.
    pub forked_from_artifact_id: Option<String>,
    pub forked_from_version: Option<i32>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A `task_artifact_versions` row: when one version was published.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ArtifactVersion {
    pub version: i32,
    pub published_at: DateTime<Utc>,
    /// The Task's newest turn when the version was published; `None` when the
    /// Task had no turn yet.
    pub turn_number: Option<i32>,
}

/// A version copied into another Task as the first version of a new Artifact.
#[derive(Debug, Clone)]
pub struct NewFork {
    pub artifact_id: String,
    pub task_id: String,
    pub slug: String,
    pub title: String,
    pub created_by: String,
    pub data: DataAccess,
    pub source_artifact_id: String,
    pub source_version: i32,
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
    pub data: DataAccess,
    /// See [`Artifact::visibility`]; `None` keeps the Artifact's current one,
    /// which a new Artifact starts `Public`.
    pub visibility: Option<Visibility>,
    /// The `<scope>:<name>` of each scoped collection the page may read,
    /// before they are resolved into [`DataAccess::collections`].
    pub collections: Vec<String>,
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
    /// The scoped record collections the page may read, each only by a
    /// reader who reaches its scope.
    pub collections: Vec<AttachedCollection>,
}

/// A scoped record collection a publish lets its page read, resolved
/// against the publishing Task's scopes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedCollection {
    pub scope: MemoryScope,
    /// The scope's id, which a reader's access to the scope is checked on.
    pub scope_id: String,
    pub name: String,
    pub collection_id: String,
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

pub use crate::collection::models::Record;
