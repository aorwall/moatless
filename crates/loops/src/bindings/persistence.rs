//! Persisted Task binding rows and domain decoding.

use chrono::{DateTime, Utc};
use moatless_types::adapter::{AdapterKind, TaskBinding};

/// Row from the `task_bindings` table.
///
/// `external_*` and `active` are `#[sqlx(default)]` since many `SELECT`s don't
/// project them; `active` defaults to `false` (fail-safe — a forgotten
/// projection drops delivery, not resurrects a Task), and live paths project
/// the real value via `TASK_BINDING_SELECT`/`upsert`'s `RETURNING`.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskBindingRow {
    pub id: String,
    pub task_id: String,
    pub adapter_kind: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub connection_id: String,
    pub loop_id: Option<String>,
    pub destination_config: serde_json::Value,
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    pub origin: String,
    pub created_at: DateTime<Utc>,
    pub last_event_at: DateTime<Utc>,
    #[sqlx(default)]
    pub external_status: Option<String>,
    #[sqlx(default)]
    pub external_state_kind: Option<String>,
    #[sqlx(default)]
    pub external_status_raw: Option<serde_json::Value>,
    #[sqlx(default)]
    pub external_review_decision: Option<String>,
    #[sqlx(default)]
    pub external_mergeable_state: Option<String>,
    #[sqlx(default)]
    pub external_status_updated_at: Option<DateTime<Utc>>,
    #[sqlx(default)]
    pub active: bool,
    #[sqlx(default)]
    pub deactivated_at: Option<DateTime<Utc>>,
    /// `None` where a `SELECT` does not project it, read as watching: the
    /// column's own default.
    #[sqlx(default)]
    pub watching: Option<bool>,
}

impl From<TaskBindingRow> for TaskBinding {
    fn from(row: TaskBindingRow) -> Self {
        Self {
            adapter_kind: AdapterKind::parse_or_webhook(&row.adapter_kind),
            id: row.id,
            task_id: row.task_id,
            subject_kind: row.subject_kind,
            subject_key: row.subject_key,
            connection_id: row.connection_id,
            loop_id: row.loop_id,
            destination_config: row.destination_config,
            metadata: row.metadata,
            last_external_event_id: row.last_external_event_id,
            origin: row.origin,
            created_at: row.created_at,
            last_event_at: row.last_event_at,
            external_status: row.external_status,
            external_state_kind: row.external_state_kind,
            external_status_raw: row.external_status_raw,
            external_review_decision: row.external_review_decision,
            external_mergeable_state: row.external_mergeable_state,
            external_status_updated_at: row.external_status_updated_at,
            active: row.active,
            deactivated_at: row.deactivated_at,
            watching: row.watching.unwrap_or(true),
        }
    }
}
