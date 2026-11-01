//! Artifact request/response types.

use serde::{Deserialize, Serialize};

use crate::enums::Visibility;

/// Largest HTML page one version may hold.
pub const MAX_PAGE_BYTES: usize = 10 * 1024 * 1024;

/// One published version of an Artifact, as `POST
/// /api/v1/tasks/{task_id}/artifacts` answers it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ArtifactResponse {
    pub artifact_id: String,
    pub task_id: String,
    pub slug: String,
    pub title: String,
    /// The latest published version, starting at 1.
    pub version: i32,
    /// Absolute URL of this version's page; never changes.
    pub url: String,
    /// Absolute URL that serves whichever version is latest.
    pub latest_url: String,
    pub created_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Whether the page may save Artifact records; set by the latest publish.
    #[serde(default)]
    pub accepts_data: bool,
    /// Whether a record a reader saves queues a message to the Task's agent.
    #[serde(default)]
    pub notify_on_data: bool,
    /// `public` lets any signed-in user read the page, whatever the Task's
    /// visibility; `private` limits it to readers of the Task. `private` from
    /// a server older than the field, which only had Task-scoped pages.
    #[serde(default = "task_scoped")]
    pub visibility: Visibility,
    /// The record rules the latest publish set, as published; `None` lets a
    /// page perform every record operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub record_rules: Option<serde_json::Value>,
    /// The scoped collections the latest publish lets the page read, as
    /// `<scope>:<name>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collections: Vec<String>,
}

fn task_scoped() -> Visibility {
    Visibility::Private
}

/// Response for `GET /api/v1/tasks/{task_id}/artifacts`, newest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListArtifactsResponse {
    pub items: Vec<ArtifactResponse>,
}

/// Doc-only type for the multipart body of `POST
/// /api/v1/tasks/{task_id}/artifacts`.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PublishArtifactBody {
    pub title: String,
    /// Stable name that later publishes of the same page reuse; derived from
    /// the title when omitted.
    pub slug: Option<String>,
    /// `true` shows this version in the Task's thread as the page itself, served
    /// with T3's theme and height bootstrap.
    pub inline: Option<bool>,
    /// The inline frame's height in CSS pixels, clamped to 80..=2000; 1200 when
    /// omitted. Ignored without `inline`.
    pub height: Option<i32>,
    /// `true` lets the page save Artifact records through the
    /// `moatless.artifact` script injected when it is served.
    pub data: Option<bool>,
    /// `true` also queues a message to the Task's agent for each record a
    /// reader saves. Ignored without `data`.
    pub notify: Option<bool>,
    /// Record rules as JSON: per key prefix, the operations a reader's page
    /// may perform and the ones that notify the agent. Needs `data`, and
    /// replaces `notify`.
    pub rules: Option<String>,
    /// `private` limits the page and its records to readers of the Task;
    /// `public` lets any signed-in user read them. Omitted keeps the
    /// Artifact's current visibility, `public` for a new one. Every version
    /// follows the latest choice.
    pub visibility: Option<Visibility>,
    /// A scoped record collection the page may read, as `<scope>:<name>`,
    /// such as `loop:review-runs`; repeat the field for more. Resolved
    /// against the publishing Task's scopes, and read only by a reader who
    /// reaches that scope.
    pub collection: Option<Vec<String>>,
    /// One self-contained page, with a `.html` or `.htm` filename.
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "binary"))]
    pub file: Vec<u8>,
}

/// Largest JSON document one Artifact record may hold, serialized.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;

/// One Artifact record: JSON saved under a key on an Artifact, kept across
/// its versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecordResponse {
    pub artifact_id: String,
    /// Names the record on its Artifact; the next save under it replaces it.
    pub key: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub data: serde_json::Value,
    /// The user id of whoever saved the record last.
    pub submitted_by: String,
    /// Whether the last write came from a page or from a user who can change
    /// the Task, such as its agent.
    #[serde(default)]
    pub writer: RecordWriter,
    pub created_at: String,
    pub updated_at: String,
}

/// Response for listing an Artifact's records, oldest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListArtifactRecordsResponse {
    pub items: Vec<ArtifactRecordResponse>,
}

/// Body of a record write; each answers with the record it wrote, or for
/// `delete`, the record as it was.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum WriteArtifactRecordRequest {
    /// Save `data` under `key`, replacing the record there.
    Save {
        key: String,
        #[cfg_attr(feature = "openapi", schema(value_type = Object))]
        data: serde_json::Value,
    },
    /// Save `data` under a new key `{list}/{id}`, where the id sorts by
    /// creation time.
    Append {
        list: String,
        #[cfg_attr(feature = "openapi", schema(value_type = Object))]
        data: serde_json::Value,
    },
    /// Remove the record under `key`.
    Delete { key: String },
}

/// Where an Artifact record's last write came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum RecordWriter {
    /// A reader's page, through its page token. A server older than the field
    /// had no other kind of record a page could tell apart.
    #[default]
    Page,
    /// A user who can change the Task, such as its agent's CLI.
    Agent,
}

impl RecordWriter {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Agent => "agent",
        }
    }
}

impl TryFrom<String> for RecordWriter {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        match value.as_str() {
            "page" => Ok(Self::Page),
            "agent" => Ok(Self::Agent),
            other => Err(format!("unknown RecordWriter: {other:?}")),
        }
    }
}

/// Request for `PUT /api/v1/artifacts/{artifact_id}/visibility`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SetArtifactVisibilityRequest {
    pub visibility: Visibility,
}

/// Which Task a fork of an Artifact version lands in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ArtifactForkMode {
    /// A fork of the Artifact's Task, cut after the turn that published the
    /// version; only for a reader of that Task.
    Task,
    /// A new, empty Task in a Workspace the caller names.
    New,
}

/// Request for `POST /api/v1/artifacts/{artifact_id}/{version}/fork`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ForkArtifactRequest {
    pub mode: ArtifactForkMode,
    /// The new Task's Workspace; required by, and only read for, `new`.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// What to change, appended to the new Task's first message.
    #[serde(default)]
    pub message: Option<String>,
}

/// Response for `POST /api/v1/artifacts/{artifact_id}/{version}/fork`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ForkArtifactResponse {
    pub task_id: String,
    /// The new Task in the web UI.
    pub task_url: String,
    /// The copy, version 1 of a private Artifact on the new Task.
    pub artifact: ArtifactResponse,
}
