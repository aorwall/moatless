//! Deserialization structs for Linear webhooks (`AgentSessionEvent`, `Issue`,
//! `Comment`, `AppUserNotification`).
//! Reference: <https://linear.app/developers/agents>.
//!
//! Also `Serialize`: the durable record stores the admitted payload verbatim,
//! since it can't be re-fetched once the body is gone. `rename` attributes
//! must hold in both directions, or a field comes back empty in the worker.

use serde::{Deserialize, Serialize};

// =============================================================================
// Envelope
// =============================================================================

/// Top-level Linear webhook envelope. Only the most-relevant common fields are
/// parsed eagerly; payload-specific fields are deserialised on demand from
/// `data` / `agentSession` / `agentActivity`.
#[derive(Debug, Clone, Deserialize)]
pub struct LinearWebhookEnvelope {
    /// `"create"` | `"update"` | `"remove"` for Issue payloads. `"created"` |
    /// `"prompted"` for `AgentSessionEvent` payloads.
    #[serde(default)]
    pub action: String,

    /// `"Issue"` | `"Comment"` | `"AgentSessionEvent"` | …
    #[serde(rename = "type", default)]
    pub event_type: String,

    /// Workspace identifier — used to route webhooks to the right
    /// `adapter_connections` row.
    #[serde(default, rename = "organizationId")]
    pub organization_id: Option<String>,

    /// Webhook-configuration id: the *stable* id of the webhook config in
    /// Linear, identical on every delivery, not unique per event — must NOT be
    /// used alone as a dedup key. See `handlers::dedup_key`.
    #[serde(default, rename = "webhookId")]
    pub webhook_id: Option<String>,

    /// Delivery timestamp (ms since epoch). Linear resends the *same* value on
    /// retries, making it a correct last-resort dedup discriminator when no
    /// resource id is present.
    #[serde(default, rename = "webhookTimestamp")]
    pub webhook_timestamp: Option<i64>,

    /// Issue payloads embed the issue here.
    #[serde(default)]
    pub data: Option<serde_json::Value>,

    /// Captured for state-change diffing on `Issue.update`.
    #[serde(default, rename = "updatedFrom")]
    pub updated_from: Option<serde_json::Value>,

    /// Actor who triggered the event (best-effort; many payloads omit it).
    #[serde(default)]
    pub actor: Option<LinearActor>,

    // ---- AgentSessionEvent-only ----
    /// Session metadata. Present on `AgentSessionEvent`.
    #[serde(default, rename = "agentSession")]
    pub agent_session: Option<AgentSession>,

    /// Prompt body. Present on `AgentSessionEvent.prompted`.
    #[serde(default, rename = "agentActivity")]
    pub agent_activity: Option<AgentActivityPayload>,

    /// `AppUserNotification` payload (Linear OAuth-app inbox stream). Not under
    /// `data` — Linear places it at the envelope's top level.
    #[serde(default)]
    pub notification: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LinearActor {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

// =============================================================================
// AgentSessionEvent
// =============================================================================

/// Linear `AgentSession` snapshot delivered with `AgentSessionEvent` payloads.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentSession {
    pub id: String,

    /// Linear issue this session is attached to.
    #[serde(default)]
    pub issue: Option<AgentSessionIssue>,

    /// Comment that triggered the session (mention). When `None` and the
    /// session was created, this is an assignment-initiated session.
    #[serde(default)]
    pub comment: Option<AgentSessionComment>,

    /// Workspace user who created the session.
    #[serde(default, rename = "creator")]
    pub creator: Option<LinearActor>,

    /// Inbound prompt context the webhook carries — a pre-built XML blob with
    /// surrounding issue context. When present, we forward it verbatim to the
    /// agent instead of doing a `fetch_context` round-trip.
    #[serde(default, rename = "promptContext")]
    pub prompt_context: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentSessionIssue {
    pub id: String,
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "teamId")]
    pub team_id: Option<String>,
    #[serde(default)]
    pub team: Option<LinearTeamRef>,
    #[serde(default)]
    pub state: Option<LinearWorkflowState>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LinearTeamRef {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LinearProjectRef {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LinearWorkflowState {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// `"unstarted"`, `"started"`, `"completed"`, `"canceled"`, etc.
    #[serde(default, rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentSessionComment {
    pub id: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub user: Option<LinearActor>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentActivityPayload {
    pub id: String,
    #[serde(default)]
    pub body: String,
}

// =============================================================================
// Comment.create — Linear's classic issue-comment event. Used by the dispatcher
// to forward follow-up comments to a task that already has an agent-session
// binding for the comment's issue.
// =============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommentPayloadData {
    pub id: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, rename = "issueId")]
    pub issue_id: Option<String>,
    #[serde(default)]
    pub issue: Option<CommentIssueRef>,
    #[serde(default)]
    pub user: Option<LinearActor>,
    #[serde(default, rename = "parentId")]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommentIssueRef {
    pub id: String,
    #[serde(default)]
    pub identifier: String,
}

impl CommentPayloadData {
    /// Resolve the issue uuid from either `issueId` or the nested `issue.id`.
    pub fn resolved_issue_id(&self) -> Option<&str> {
        if let Some(id) = self.issue_id.as_deref() {
            return Some(id);
        }
        self.issue.as_ref().map(|i| i.id.as_str())
    }
}

// =============================================================================
// Classic Issue payload (slim — we only need state + identity for wake/close)
// =============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IssuePayload {
    pub id: String,
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "teamId")]
    pub team_id: Option<String>,
    #[serde(default)]
    pub team: Option<LinearTeamRef>,
    /// Project the issue belongs to (scalar id on the webhook payload).
    #[serde(default, rename = "projectId")]
    pub project_id: Option<String>,
    /// Nested project object, when the webhook embeds it.
    #[serde(default)]
    pub project: Option<LinearProjectRef>,
    /// Linear issue update timestamp. Duplicate webhook deliveries for one
    /// persisted issue mutation can carry different webhook timestamps, but the
    /// issue's own updatedAt remains the event identity.
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub state: Option<LinearWorkflowState>,
    /// The issue's current label ids. Paired with `updatedFrom.labelIds` to
    /// tell which labels an update added.
    #[serde(default, rename = "labelIds")]
    pub label_ids: Vec<String>,
    /// The issue's current labels, when the webhook embeds them.
    #[serde(default)]
    pub labels: Vec<LinearLabelRef>,
}

/// A label as the `Issue` webhook embeds it.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LinearLabelRef {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

impl IssuePayload {
    /// Resolve the team uuid from either `teamId` or the nested `team.id`.
    pub fn resolved_team_id(&self) -> Option<&str> {
        if let Some(id) = self.team_id.as_deref() {
            return Some(id);
        }
        self.team.as_ref().map(|t| t.id.as_str())
    }

    /// Resolve the project uuid from either `projectId` or the nested
    /// `project.id`. `None` when the issue has no project.
    pub fn resolved_project_id(&self) -> Option<&str> {
        if let Some(id) = self.project_id.as_deref() {
            return Some(id);
        }
        self.project.as_ref().map(|p| p.id.as_str())
    }

    /// Resolve the state name (empty string when missing).
    pub fn state_name(&self) -> &str {
        self.state.as_ref().map_or("", |s| s.name.as_str())
    }

    /// Resolve the state type — Linear's stable enum (`"backlog" | "unstarted"
    /// | "started" | "completed" | "canceled" | "triage"`), empty when missing.
    /// Prefer over `state_name` for matching: names are user-editable per
    /// workspace.
    pub fn state_kind(&self) -> &str {
        self.state.as_ref().map_or("", |s| s.kind.as_str())
    }
}

// =============================================================================
// AppUserNotification — Linear's OAuth-app inbox stream; the notification
// body is at the envelope's top-level `notification` field. Only
// `issueStatusChanged` (close-on-terminal) does anything; the rest no-op.
// =============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppUserNotificationData {
    #[serde(default)]
    pub id: Option<String>,
    /// `issueStatusChanged` | `issueMention` | `issueCommentMention`
    /// | `issueAssignedToYou` | `issueNewComment` | ...
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default, rename = "issueId")]
    pub issue_id: Option<String>,
    #[serde(default)]
    pub issue: Option<IssuePayload>,
    #[serde(default)]
    pub actor: Option<LinearActor>,
}

impl AppUserNotificationData {
    /// Resolve the carried issue payload (only present on issue-scoped
    /// notification kinds).
    pub fn resolved_issue(&self) -> Option<&IssuePayload> {
        self.issue.as_ref()
    }
}

/// The name Linear knows an actor by, for the messages they write.
///
/// Linear defaults every actor field, so an unnamed event's actor arrives as an
/// empty name, not `None`.
pub(crate) fn actor_handle(actor: Option<&LinearActor>) -> Option<String> {
    actor.and_then(|actor| moatless_types::adapter::author_handle(&actor.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_session_created_with_mention() {
        let raw = serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "organizationId": "org-uuid",
            "webhookId": "wh-1",
            "agentSession": {
                "id": "session-1",
                "issue": {
                    "id": "issue-1",
                    "identifier": "ENG-1",
                    "title": "Bug",
                    "teamId": "team-1",
                    "state": {"id": "st-1", "name": "Todo", "type": "unstarted"},
                    "url": "https://linear.app/acme/issue/ENG-1"
                },
                "comment": {
                    "id": "comment-1",
                    "body": "@agent please investigate",
                    "user": {"id": "user-1", "name": "Alice"}
                },
                "promptContext": "<context></context>"
            }
        });
        let env: LinearWebhookEnvelope = serde_json::from_value(raw).expect("parse");
        assert_eq!(env.event_type, "AgentSessionEvent");
        assert_eq!(env.action, "created");
        let session = env.agent_session.expect("session present");
        assert_eq!(session.id, "session-1");
        let comment = session.comment.expect("comment present");
        assert_eq!(comment.body, "@agent please investigate");
        let issue = session.issue.expect("issue present");
        assert_eq!(issue.identifier, "ENG-1");
        assert_eq!(issue.state.expect("state").name, "Todo");
    }

    #[test]
    fn parses_agent_session_created_from_assignment() {
        let raw = serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "organizationId": "org-uuid",
            "agentSession": {
                "id": "session-2",
                "issue": {
                    "id": "issue-2",
                    "identifier": "ENG-2",
                    "title": "Feature",
                    "teamId": "team-1",
                    "state": {"id": "st-2", "name": "Backlog", "type": "backlog"}
                }
            }
        });
        let env: LinearWebhookEnvelope = serde_json::from_value(raw).expect("parse");
        let session = env.agent_session.expect("session present");
        assert!(
            session.comment.is_none(),
            "assignment-initiated session has no comment"
        );
    }

    #[test]
    fn parses_agent_session_prompted() {
        let raw = serde_json::json!({
            "action": "prompted",
            "type": "AgentSessionEvent",
            "organizationId": "org-uuid",
            "agentSession": {"id": "session-3"},
            "agentActivity": {
                "id": "act-1",
                "type": "prompt",
                "body": "follow up message"
            }
        });
        let env: LinearWebhookEnvelope = serde_json::from_value(raw).expect("parse");
        assert_eq!(env.action, "prompted");
        let activity = env.agent_activity.expect("activity present");
        assert_eq!(activity.body, "follow up message");
    }

    #[test]
    fn parses_issue_update_with_state_change() {
        let raw = serde_json::json!({
            "action": "update",
            "type": "Issue",
            "organizationId": "org-uuid",
            "updatedFrom": {"stateId": "st-old"},
            "data": {
                "id": "issue-3",
                "identifier": "ENG-3",
                "title": "Refactor",
                "teamId": "team-1",
                "state": {"id": "st-new", "name": "In Progress", "type": "started"}
            }
        });
        let env: LinearWebhookEnvelope = serde_json::from_value(raw).expect("parse");
        assert_eq!(env.event_type, "Issue");
        let data = env.data.expect("data present");
        let issue: IssuePayload = serde_json::from_value(data).expect("parse issue");
        assert_eq!(issue.resolved_team_id(), Some("team-1"));
        assert_eq!(issue.state_name(), "In Progress");
        assert_eq!(issue.state_kind(), "started");
    }

    #[test]
    fn parses_issue_update_with_project_scalar_and_nested() {
        // Scalar `projectId` wins; nested `project.id` is the fallback.
        let scalar: IssuePayload = serde_json::from_value(serde_json::json!({
            "id": "issue-p1",
            "identifier": "ENG-P1",
            "title": "Scoped",
            "teamId": "team-1",
            "projectId": "proj-123",
            "state": {"id": "st", "name": "Ready", "type": "unstarted"}
        }))
        .expect("parse");
        assert_eq!(scalar.resolved_project_id(), Some("proj-123"));

        let nested: IssuePayload = serde_json::from_value(serde_json::json!({
            "id": "issue-p2",
            "identifier": "ENG-P2",
            "title": "Scoped",
            "teamId": "team-1",
            "project": {"id": "proj-456", "name": "Launch"},
        }))
        .expect("parse");
        assert_eq!(nested.resolved_project_id(), Some("proj-456"));

        let none: IssuePayload = serde_json::from_value(serde_json::json!({
            "id": "issue-p3",
            "identifier": "ENG-P3",
            "title": "Unscoped",
            "teamId": "team-1"
        }))
        .expect("parse");
        assert_eq!(none.resolved_project_id(), None);
    }

    #[test]
    fn state_kind_missing_returns_empty() {
        let issue: IssuePayload = serde_json::from_value(serde_json::json!({
            "id": "issue-x",
            "identifier": "ENG-X",
            "title": "Untyped",
            "teamId": "team-1"
        }))
        .expect("parse");
        assert_eq!(issue.state_kind(), "");
        assert_eq!(issue.state_name(), "");
    }

    #[test]
    fn parses_comment_create_with_issue_id() {
        let raw = serde_json::json!({
            "action": "create",
            "type": "Comment",
            "organizationId": "org-uuid",
            "data": {
                "id": "comment-100",
                "body": "any update here?",
                "issueId": "issue-12",
                "user": {"id": "user-2", "name": "Bob"}
            }
        });
        let env: LinearWebhookEnvelope = serde_json::from_value(raw).expect("parse");
        assert_eq!(env.event_type, "Comment");
        assert_eq!(env.action, "create");
        let data = env.data.expect("data present");
        let comment: CommentPayloadData = serde_json::from_value(data).expect("parse comment");
        assert_eq!(comment.id, "comment-100");
        assert_eq!(comment.body, "any update here?");
        assert_eq!(comment.resolved_issue_id(), Some("issue-12"));
        assert!(comment.parent_id.is_none());
    }

    #[test]
    fn parses_app_user_notification_issue_status_changed() {
        let raw = serde_json::json!({
            "action": "issueStatusChanged",
            "type": "AppUserNotification",
            "organizationId": "org-uuid",
            "webhookId": "wh-aun-1",
            "notification": {
                "id": "notif-1",
                "type": "issueStatusChanged",
                "issueId": "issue-9",
                "issue": {
                    "id": "issue-9",
                    "identifier": "ENG-9",
                    "title": "Done thing",
                    "teamId": "team-1",
                    "state": {"id": "st-done", "name": "Done", "type": "completed"},
                    "url": "https://linear.app/acme/issue/ENG-9"
                },
                "actor": {"id": "user-1", "name": "Alice"}
            }
        });
        let env: LinearWebhookEnvelope = serde_json::from_value(raw).expect("parse");
        assert_eq!(env.event_type, "AppUserNotification");
        assert_eq!(env.action, "issueStatusChanged");
        let raw_notif = env.notification.expect("notification present");
        let notif: AppUserNotificationData =
            serde_json::from_value(raw_notif).expect("parse notification");
        assert_eq!(notif.kind, "issueStatusChanged");
        assert_eq!(notif.id.as_deref(), Some("notif-1"));
        assert_eq!(notif.issue_id.as_deref(), Some("issue-9"));
        let issue = notif.resolved_issue().expect("issue present");
        assert_eq!(issue.identifier, "ENG-9");
        assert_eq!(issue.state_kind(), "completed");
        assert_eq!(issue.resolved_team_id(), Some("team-1"));
        assert_eq!(notif.actor.expect("actor").id, "user-1");
    }

    #[test]
    fn parses_app_user_notification_unknown_action_payload() {
        // Sub-actions like `issueAssignedToYou` are routed but not acted on —
        // the handler logs and returns Ok. Make sure the payload still parses.
        let raw = serde_json::json!({
            "action": "issueAssignedToYou",
            "type": "AppUserNotification",
            "organizationId": "org-uuid",
            "notification": {
                "id": "notif-2",
                "type": "issueAssignedToYou",
                "issueId": "issue-10"
            }
        });
        let env: LinearWebhookEnvelope = serde_json::from_value(raw).expect("parse");
        let notif: AppUserNotificationData =
            serde_json::from_value(env.notification.expect("notif")).expect("parse");
        assert_eq!(notif.kind, "issueAssignedToYou");
        assert!(notif.resolved_issue().is_none());
    }

    #[test]
    fn parses_comment_create_with_nested_issue() {
        let raw = serde_json::json!({
            "action": "create",
            "type": "Comment",
            "organizationId": "org-uuid",
            "data": {
                "id": "comment-101",
                "body": "reply",
                "issue": {"id": "issue-13", "identifier": "ENG-13"},
                "parentId": "comment-100"
            }
        });
        let env: LinearWebhookEnvelope = serde_json::from_value(raw).expect("parse");
        let data = env.data.expect("data present");
        let comment: CommentPayloadData = serde_json::from_value(data).expect("parse comment");
        assert!(comment.issue_id.is_none());
        assert_eq!(comment.resolved_issue_id(), Some("issue-13"));
        let issue = comment.issue.expect("nested issue");
        assert_eq!(issue.id, "issue-13");
        assert_eq!(issue.identifier, "ENG-13");
        assert_eq!(comment.parent_id.as_deref(), Some("comment-100"));
    }
}

#[cfg(test)]
mod actor_tests {
    use super::{LinearActor, actor_handle};

    fn actor(name: &str) -> LinearActor {
        LinearActor {
            id: "user-uuid".to_string(),
            name: name.to_string(),
            email: None,
        }
    }

    /// An event with no actor at all, and one whose actor was defaulted into
    /// existence by an empty payload, both name nobody.
    #[test]
    fn an_event_that_names_nobody_yields_no_handle() {
        assert_eq!(actor_handle(Some(&actor("Ada"))).as_deref(), Some("Ada"));
        assert_eq!(actor_handle(Some(&actor(""))), None);
        assert_eq!(actor_handle(None), None);
    }
}
