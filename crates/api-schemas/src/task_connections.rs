//! Admin task-connections view: tasks with their external connections
//! (GitHub PR / Linear issue / Slack thread), each connection's external
//! status, and per-connection inbound/outgoing message counts.

use serde::{Deserialize, Serialize};

use crate::enums::{AdapterKind, TaskStatus};
use crate::task::PaginationResponse;

/// Query parameters for `GET /api/v1/admin/task-connections`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[serde(rename_all = "camelCase")]
pub struct ListTaskConnectionsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// When `true`, fill in missing/stale connection statuses with a live
    /// GitHub / Linear fetch (bounded concurrency). Default is DB-only and
    /// fast.
    pub refresh: Option<bool>,
}

/// Query parameters for `GET /api/v1/tasks/{task_id}/connections`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[serde(rename_all = "camelCase")]
pub struct GetTaskConnectionsQuery {
    /// When `true`, fill in missing/stale connection statuses with a live
    /// GitHub / Linear fetch (bounded concurrency). Default is DB-only and
    /// fast.
    pub refresh: Option<bool>,
}

/// Paginated list of tasks with their external connections.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskConnectionsResponse {
    pub items: Vec<TaskConnectionRow>,
    pub pagination: PaginationResponse,
}

/// One task and the external connections wired to it.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskConnectionRow {
    pub task_id: String,
    pub short_id: Option<String>,
    pub title: String,
    pub status: TaskStatus,
    pub repository_name: Option<String>,
    /// Display name of the task's creator (`None` when unresolved).
    pub owner: Option<String>,
    pub connections: Vec<Connection>,
}

/// A single external connection (one task binding) with its decoded label,
/// external status, and message tally.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub binding_id: String,
    pub adapter_kind: AdapterKind,
    /// Decoded connection kind: `github_pr` | `linear_issue` | `slack_thread`
    /// | the raw `subject_kind` for anything else.
    pub kind: String,
    /// Human-readable label such as `owner/repo#42` or `SP-1721`.
    pub label: String,
    /// Web URL for the connected subject, when derivable.
    pub url: Option<String>,
    /// Display status string (GitHub: open/merged/closed/draft; Linear: state
    /// name). `None` when no status has been persisted/fetched yet.
    pub status: Option<String>,
    /// Normalized status bucket for color-coding.
    pub state_kind: Option<String>,
    /// RFC3339 timestamp of when the status was last refreshed.
    pub status_updated_at: Option<String>,
    /// Inbound message count for this connection.
    pub incoming: i64,
    /// Outgoing message count for this connection.
    pub outgoing: i64,
    /// RFC3339 timestamp of the last event seen on the binding.
    pub last_event_at: Option<String>,
    /// Whether this connection still routes inbound events. `false` once the
    /// task closes — the binding is kept for history but stops listening.
    pub active: bool,
    /// RFC3339 timestamp of when the connection was deactivated; `None` while
    /// active.
    pub deactivated_at: Option<String>,
}
