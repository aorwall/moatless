use chrono::{DateTime, Utc};
use serde_json::Value;
#[cfg(feature = "database")]
use sqlx::{postgres::PgRow, Row};

use super::mapping::{ActivityKind, ActivityStatus};

/// One stored activity.
#[derive(Debug, Clone)]
pub struct Activity {
    pub activity_id: String,
    pub task_id: String,
    pub position: i64,
    /// Where this row sits inside its change. See
    /// [`ActivityRow::ordinal`](ActivityRow::ordinal).
    pub ordinal: i32,
    pub message_uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub kind: ActivityKind,
    pub status: ActivityStatus,
    pub tool_use_id: Option<String>,
    pub summary: String,
    pub payload: Value,
    /// What a settled call shows before it is expanded. `None` until the call
    /// comes back, and on every activity that is not a tool call.
    pub detail: Option<String>,
    pub is_error: bool,
    pub external_event_id: Option<String>,
    pub external_adapter: Option<String>,
    pub external_source_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Activity {
    /// The tool's input as the call carried it, or `Null` for an activity that
    /// is not a tool call.
    pub fn tool_input(&self) -> &Value {
        self.payload.get("input").unwrap_or(&Value::Null)
    }

    /// The result whole, or `None` while the call is still in progress.
    pub fn result(&self) -> Option<&Value> {
        self.payload.get("result")
    }

    /// The transport's structured view of the outcome, where it reported one.
    pub fn typed_result(&self) -> Option<&Value> {
        self.payload.get("typedResult")
    }

    /// Whether the call this activity records has come back.
    pub fn is_settled(&self) -> bool {
        self.status != ActivityStatus::InProgress
    }
}

#[cfg(feature = "database")]
impl sqlx::FromRow<'_, PgRow> for Activity {
    fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        let kind: String = row.try_get("kind")?;
        let status: String = row.try_get("status")?;
        Ok(Self {
            activity_id: row.try_get("activity_id")?,
            task_id: row.try_get("task_id")?,
            position: row.try_get("position")?,
            ordinal: row.try_get("ordinal")?,
            message_uuid: row.try_get("message_uuid")?,
            parent_uuid: row.try_get("parent_uuid")?,
            kind: kind
                .parse()
                .map_err(|e: String| sqlx::Error::ColumnDecode {
                    index: "kind".to_string(),
                    source: e.into(),
                })?,
            status: match status.as_str() {
                "in_progress" => ActivityStatus::InProgress,
                "completed" => ActivityStatus::Completed,
                "failed" => ActivityStatus::Failed,
                other => {
                    return Err(sqlx::Error::ColumnDecode {
                        index: "status".to_string(),
                        source: format!("unknown activity status: {other:?}").into(),
                    });
                }
            },
            tool_use_id: row.try_get("tool_use_id")?,
            summary: row.try_get("summary")?,
            payload: row.try_get("payload")?,
            detail: row.try_get("detail")?,
            is_error: row.try_get("is_error")?,
            external_event_id: row.try_get("external_event_id")?,
            external_adapter: row.try_get("external_adapter")?,
            external_source_name: row.try_get("external_source_name")?,
            created_at: row.try_get("created_at")?,
        })
    }
}
