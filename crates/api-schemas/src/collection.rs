//! Record collection request/response types.
//!
//! The agent's routes sit under `/api/v1/tasks/{task_id}/collections`: the
//! Task names the scope ids, as for Memory, so a request names only a
//! [`MemoryScope`]. A write body is a [`crate::artifact::WriteArtifactRecordRequest`].

use serde::{Deserialize, Serialize};

pub use crate::artifact::RecordWriter;
pub use moatless_types::memory::MemoryScope;

/// Longest collection name, in bytes.
pub const MAX_COLLECTION_NAME_LEN: usize = 63;

/// One record in a scoped collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CollectionRecordResponse {
    /// The collection's name in its scope.
    pub collection: String,
    pub scope: MemoryScope,
    /// Names the record in its collection; the next save under it replaces it.
    pub key: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub data: serde_json::Value,
    /// The user id of whoever saved the record last.
    pub submitted_by: String,
    #[serde(default)]
    pub writer: RecordWriter,
    pub created_at: String,
    pub updated_at: String,
}

/// Response for listing a collection's records, oldest change first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListCollectionRecordsResponse {
    pub items: Vec<CollectionRecordResponse>,
}

/// One collection in a scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CollectionResponse {
    pub name: String,
    pub scope: MemoryScope,
    pub record_count: i64,
    /// When its newest record changed; `None` while it holds none.
    pub last_updated_at: Option<String>,
    pub created_at: String,
}

/// Response for listing a scope's collections, by name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListCollectionsResponse {
    pub items: Vec<CollectionResponse>,
}
