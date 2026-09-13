//! Provider outbound messages, destinations, and task attribution.

use moatless_types::adapter::OutboundPresentation;
use serde::{Deserialize, Serialize};

/// Resolved file attachment for outbound adapter messages.
#[derive(Clone, Debug)]
pub struct OutboundFile {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

/// Where a reply came from, rendered as a footer by adapters that link back to
/// the task themselves.
///
/// Linear/GitHub PR don't use this (their Markdown bodies carry the link via
/// [`OutboundReply::footer`]/`render_footer`); Slack mrkdwn has no
/// `[text](url)`, so these pieces travel structured instead. Every field but
/// `url` is droppable — a missing workspace or task name just omits its
/// segment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskProvenance {
    /// `{frontend_url}/tasks/{task_id}`.
    pub url: String,
    pub task_name: Option<String>,
    pub workspace_name: Option<String>,
    pub origin: TaskOrigin,
}

impl TaskProvenance {
    /// Text for the link itself: the task's name, or a generic fallback — the
    /// same rule `render_footer` uses for Linear and GitHub.
    pub fn label(&self) -> &str {
        self.task_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("View task")
    }

    /// The segments that follow the link, in order, skipping what is unknown.
    /// Adapters join these with their own separator so one adapter's markup
    /// never leaks into another's.
    pub fn details(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(ws) = self
            .workspace_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.push(ws.to_string());
        }
        match &self.origin {
            TaskOrigin::Loop { name } => out.push(format!("loop: {name}")),
            TaskOrigin::StartedBy { login } => out.push(format!("started by {login}")),
            TaskOrigin::Unknown => {}
        }
        out
    }
}

/// What set a task going.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskOrigin {
    /// Fired by a Loop, named by `loops.name`.
    Loop { name: String },
    /// Started by a person, named by their moatless login.
    StartedBy { login: String },
    /// Neither is known — the task predates provenance, or both lookups failed.
    Unknown,
}

/// Outbound reply to send back to an external target.
#[derive(Debug)]
pub struct OutboundReply {
    pub text: String,
    /// Provenance footer for adapters that render the task link themselves
    /// (Slack). `None` leaves the reply without one.
    pub provenance: Option<TaskProvenance>,
    /// Footer suffix appended to comment bodies by the Linear/GitHub adapters
    /// (e.g. `"\n\n---\nMoatless task: [Task name](https://app/tasks/<id>)"`),
    /// built by the binding layer's `task_comment_footer`. Its URL doubles as
    /// the inbound echo-filter source; `None` for providers without a footer.
    pub footer: Option<String>,
    /// Preferred system user whose Adapter credentials should author this reply
    /// — the owning task's `owner_user_id`. Providers resolving a per-user
    /// credential (currently GitHub) prefer this over binding/trigger
    /// identities, so a human-owned task replies under the human and a
    /// trigger-created task replies under its run-as bot. `None` when no task
    /// owner is in scope or unused.
    pub author_user_id: Option<String>,
    pub is_error: bool,
    pub files: Vec<OutboundFile>,
    /// Adapter-native rich rendering of this reply, when the caller supplied
    /// one. `None` reproduces the pre-presentation behaviour exactly.
    pub presentation: Option<OutboundPresentation>,
}

/// Result of posting to a provider-specific destination by raw ID.
///
/// Returned by `AdapterDirectDestination::post_to_destination` and used to
/// build a matching `TaskBinding` row when the post is associated with a task.
#[derive(Debug, Clone)]
pub struct DestinationPostResult {
    /// Provider-specific external message ID (e.g. Slack message `ts`).
    pub message_external_id: String,
    /// Provider-specific binding `subject_kind`.
    pub subject_kind: &'static str,
    /// Binding `subject_key` matching the format produced during inbound
    /// routing for this provider.
    pub subject_key: String,
    /// Binding `destination_config` JSON used by `send_reply_for_binding`.
    pub destination_config: serde_json::Value,
}
