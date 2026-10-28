//! Record collection row types.

use chrono::{DateTime, Utc};
use moatless_api_schemas::artifact::RecordWriter;

/// A `collection_records` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Record {
    /// For an Artifact's own records, the Artifact's id.
    pub collection_id: String,
    pub key: String,
    pub data: serde_json::Value,
    pub submitted_by: String,
    #[sqlx(try_from = "String")]
    pub writer: RecordWriter,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A `record_collections` row, with how many records it holds and when the
/// newest of them changed.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CollectionRow {
    pub collection_id: String,
    pub scope_kind: String,
    pub scope_id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub record_count: i64,
    pub last_updated_at: Option<DateTime<Utc>>,
}

/// The `scope_kind` of an Artifact's own collection.
pub const ARTIFACT_SCOPE_KIND: &str = "artifact";
