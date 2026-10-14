//! The row a search returns.

use chrono::{DateTime, Utc};

/// One open Task a search matched, and the text that matched in it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskMatch {
    pub task_id: String,
    /// `None` for older and hand-written rows, as on [`crate::task_listing::ListedTask`]. A match
    /// without one is dropped by the T3 projection: the contract's match
    /// carries a project id and the client uses it to route.
    pub workspace_id: Option<String>,
    /// Whether a person or the agent wrote the matched text. A Task *name*
    /// matched is reported as the person's — it is Moatless's handle for the
    /// Task and the contract has no third source to call it.
    pub from_user: bool,
    /// What matched: a Message's text, or the Task's name.
    pub matched_text: String,
    /// When the matched Message was written. `None` for a name match, which the
    /// contract allows precisely so a match need not be a Message.
    pub matched_at: Option<DateTime<Utc>>,
}
