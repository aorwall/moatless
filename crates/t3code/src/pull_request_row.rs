//! The pull requests a [`crate::dao::Conversation`] row aggregates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One GitHub pull request bound to a Task, as the listing query aggregates it.
///
/// `subject_key` alone makes a row reportable, linking from the moment it's
/// written; `title`, `base_ref` and `head_ref` come from a refresh's status
/// blob and are `None` together until one runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationPullRequest {
    pub repository: String,
    pub number: i64,
    pub url: String,
    pub title: Option<String>,
    pub base_ref: Option<String>,
    pub head_ref: Option<String>,
    /// The binding's normalized state kind, in its own column rather than the
    /// blob: `open`, `draft`, `merged` or `closed`, and `None` until the
    /// adapter has classified one. Read by `t3_pr_state` in [`crate::ports`].
    pub state_kind: Option<String>,
    /// When this deployment last read the pull request's state from GitHub,
    /// falling back to the binding's `NOT NULL` `last_event_at` so it resolves
    /// for an unrefreshed binding too.
    ///
    /// The column [`crate::ports::BoundPullRequest::status_read_at`] reports.
    pub status_read_at: DateTime<Utc>,
    /// When the binding was written. T3's `linkedAt`, and what orders links of
    /// equal standing there.
    pub linked_at: DateTime<Utc>,
    /// `task_bindings.origin`, translated to a T3 link source by
    /// [`crate::projection::pull_request::thread_pull_request_source`].
    pub origin: String,
    /// Latest review verdict, GitHub's spelling. `None` until one is recorded.
    pub review_decision: Option<String>,
    /// GitHub's coarse mergeable state. `None` until it is computed.
    pub mergeable_state: Option<String>,
    /// Whether the pull request's events wake the agent. Defaulted for a blob
    /// written before the column existed, which every Binding then had on.
    #[serde(default = "watching_by_default")]
    pub watching: bool,
}

fn watching_by_default() -> bool {
    true
}
