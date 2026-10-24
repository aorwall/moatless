//! Memory request/response types.
//!
//! Every route sits under `/api/v1/tasks/{task_id}/memory`: the Task names the
//! scope ids, so a request only ever names a [`MemoryScope`]. A request
//! reaches that one scope, or the Task's Loop, else the Task, when it names
//! none.

use serde::{Deserialize, Serialize};

pub use moatless_types::memory::{MAX_MEMORY_KEY_LEN, MemoryOp, MemoryScope};

/// Longest `summary`, in characters.
pub const MAX_SUMMARY_CHARS: usize = 160;
/// Largest `body`, and largest serialized `data`, in bytes.
pub const MAX_BODY_BYTES: usize = 16 * 1024;
/// Lifetime of an entry written without `ttlSeconds` or `noExpiry`, renewed by
/// every put and verify.
pub const DEFAULT_TTL_SECONDS: i64 = 28 * 24 * 60 * 60;

/// Body of `PUT /api/v1/tasks/{task_id}/memory/entry`. An omitted field keeps
/// the entry's current value; a new entry needs `summary`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PutMemoryRequest {
    pub key: String,
    /// The one scope the write lands in; the Task's Loop, else the Task, when
    /// omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<MemoryScope>,
    /// One line saying what the entry is about; listings show only this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Sources the entry rests on: `path:line`, `linear:SP-1`, `task:<id>`, a URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citations: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<i64>,
    /// The entry never expires. Conflicts with `ttlSeconds`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_expiry: bool,
    /// Reads mark the entry stale this long after its last put or verify.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_after_seconds: Option<i64>,
    /// Refuse the write with 409 unless the entry is at this version; 0 means
    /// it must not exist yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_version: Option<i32>,
}

/// Body naming one entry, for verify and release.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct MemoryKeyRequest {
    pub key: String,
    /// The one scope the entry is in; the Task's Loop, else the Task, when
    /// omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<MemoryScope>,
}

/// Body of `POST /api/v1/tasks/{task_id}/memory/append`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AppendMemoryRequest {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<MemoryScope>,
    pub text: String,
}

/// Body of `POST /api/v1/tasks/{task_id}/memory/claim`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ClaimMemoryRequest {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<MemoryScope>,
    pub ttl_seconds: i64,
}

/// A whole entry, as a read returns it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntryResponse {
    pub key: String,
    pub scope: MemoryScope,
    pub summary: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    pub tags: Vec<String>,
    pub citations: Vec<String>,
    pub version: i32,
    /// `staleAfterSeconds` has passed since the last put or verify.
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
    /// The open Task holding a live claim on the entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_by_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by_task_id: Option<String>,
    /// The Task whose put created the entry; never changed by later writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_loop_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// An entry without its body or data, as listings return it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct MemorySummaryResponse {
    pub key: String,
    pub scope: MemoryScope,
    pub summary: String,
    pub tags: Vec<String>,
    pub version: i32,
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_by_task_id: Option<String>,
    pub updated_at: String,
}

/// Response of `GET /api/v1/tasks/{task_id}/memory/entry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GetMemoryResponse {
    pub items: Vec<MemoryEntryResponse>,
}

/// Response of the list and search routes, newest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListMemoryResponse {
    pub items: Vec<MemorySummaryResponse>,
}

/// One write in an entry's history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct MemoryEventResponse {
    pub op: MemoryOp,
    /// The entry's version once the write landed.
    pub version: i32,
    /// The appended text, or the summary a put left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub created_at: String,
}

/// Response of `GET /api/v1/tasks/{task_id}/memory/log`, newest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct MemoryLogResponse {
    pub key: String,
    pub scope: MemoryScope,
    pub items: Vec<MemoryEventResponse>,
}
