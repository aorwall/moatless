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
    /// One self-contained page, with a `.html` or `.htm` filename.
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "binary"))]
    pub file: Vec<u8>,
}
