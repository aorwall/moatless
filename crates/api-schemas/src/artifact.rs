//! Artifact request/response types.

use serde::{Deserialize, Serialize};

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
    /// Absolute URL that redirects to whichever version is latest.
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
    /// One self-contained page, with a `.html` or `.htm` filename.
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "binary"))]
    pub file: Vec<u8>,
}

/// Largest JSON document one Artifact record may hold, serialized.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;

/// One Artifact record: JSON saved against an Artifact, kept across its
/// versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecordResponse {
    pub record_id: String,
    pub artifact_id: String,
    /// `None` for an appended record; a keyed record is replaced by the next
    /// save under its key.
    pub key: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub data: serde_json::Value,
    /// The user id of whoever saved the record last.
    pub submitted_by: String,
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

/// Body of a record save.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SaveArtifactRecordRequest {
    /// Replace the record saved under this key rather than append one.
    #[serde(default)]
    pub key: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub data: serde_json::Value,
}
