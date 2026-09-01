//! Task vocabulary: the Task itself, the provenance a Message carries into
//! one, and what a Task's and a Turn's events say.

// reason: test code may unwrap freely; production code keeps the workspace `deny`.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::access_control::{Scope, ScopedResource};
use crate::agent::AgentType;

/// The provenance that says a Message reached this Task from outside it,
/// carried as one value rather than as six positional arguments. An internal
/// send — a person in the composer, task creation — passes
/// [`ExternalOrigin::none`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ExternalOrigin<'a> {
    pub event_id: Option<&'a str>,
    pub adapter: Option<&'a str>,
    pub subject_key: Option<&'a str>,
    pub source_name: Option<&'a str>,
    /// The handle the external system knows the writer by, recorded verbatim
    /// and never resolved to a Moatless identity.
    pub author: Option<&'a str>,
    /// The writer's own id in the external system — a Slack `U0000000001`.
    /// What the sender label resolves through, since the Moatless user id on
    /// the send is the subscription's owner when the writer is unmapped.
    pub sender_id: Option<&'a str>,
    pub source_task_id: Option<&'a str>,
}

impl ExternalOrigin<'_> {
    /// The origin of a Message that came from inside Moatless.
    pub const fn none() -> Self {
        Self {
            event_id: None,
            adapter: None,
            subject_key: None,
            source_name: None,
            author: None,
            sender_id: None,
            source_task_id: None,
        }
    }
}

/// Directive carried from a task-creation site to record a `related_to`
/// relation immediately after the new task is inserted. Use this when a
/// webhook spawned the new task and we discovered a peer task that shares the
/// same external artifact (binding subject_key) — the peer is not a parent.
#[derive(Debug, Clone)]
pub struct RelatedTask {
    pub related_task_id: String,
    pub context: serde_json::Value,
}

/// Task sub-types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TaskSubType {
    Created,
    Open,
    Closed,
    Updated,
    #[serde(rename = "metadata.updated")]
    MetadataUpdated,
}

impl TaskSubType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Updated => "updated",
            Self::MetadataUpdated => "metadata.updated",
        }
    }
}

impl std::fmt::Display for TaskSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for TaskSubType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "created" => Ok(Self::Created),
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            "updated" => Ok(Self::Updated),
            "metadata.updated" => Ok(Self::MetadataUpdated),
            other => Err(format!("unknown TaskSubType: {other}")),
        }
    }
}

/// TaskTurn sub-types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TaskTurnSubType {
    Active,
    Completed,
    Error,
    Stopped,
    /// The turn's sandbox pod terminated involuntarily (eviction, node loss)
    /// while the turn was active. Emitted immediately by the pod watcher so the
    /// interruption is visible without waiting for the delayed orphaned-turn
    /// reconciliation. The turn resumes once the pod reschedules — not a
    /// terminal error, so `is_error` stays false.
    ///
    /// Renamed explicitly: `rename_all = "snake_case"` would yield
    /// `"interrupted"`, but the wire value (and `as_str`/`TryFrom`/frontend) is
    /// `"turn_interrupted"` — pin it so the serde-derived OpenAPI schema agrees.
    #[serde(rename = "turn_interrupted")]
    Interrupted,
}

impl TaskTurnSubType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Stopped => "stopped",
            Self::Interrupted => "turn_interrupted",
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error)
    }
}

impl std::fmt::Display for TaskTurnSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for TaskTurnSubType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "active" => Ok(Self::Active),
            "completed" => Ok(Self::Completed),
            "error" => Ok(Self::Error),
            "stopped" => Ok(Self::Stopped),
            "turn_interrupted" => Ok(Self::Interrupted),
            other => Err(format!("unknown TaskTurnSubType: {other}")),
        }
    }
}

/// Task event payload — typed shape for `EventType::Task` events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskEventPayload {
    #[serde(default)]
    pub message: String,
}

/// TaskTurn event payload — typed shape for `EventType::TaskTurn` events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskTurnEventPayload {
    #[serde(default)]
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::SubType;

    #[test]
    fn task_sub_type_metadata_updated() {
        assert_eq!(
            serde_json::to_string(&TaskSubType::MetadataUpdated).unwrap(),
            "\"metadata.updated\""
        );
    }

    #[test]
    fn task_sub_type_try_from() {
        assert_eq!(
            TaskSubType::try_from("metadata.updated"),
            Ok(TaskSubType::MetadataUpdated)
        );
        assert_eq!(TaskSubType::try_from("open"), Ok(TaskSubType::Open));
        assert!(TaskSubType::try_from("nope").is_err());
    }

    #[test]
    fn task_turn_sub_type_try_from() {
        assert_eq!(
            TaskTurnSubType::try_from("completed"),
            Ok(TaskTurnSubType::Completed)
        );
        assert_eq!(
            TaskTurnSubType::try_from("stopped"),
            Ok(TaskTurnSubType::Stopped)
        );
        assert_eq!(
            TaskTurnSubType::try_from("turn_interrupted"),
            Ok(TaskTurnSubType::Interrupted)
        );
        assert!(TaskTurnSubType::try_from("x").is_err());
    }

    #[test]
    fn task_turn_interrupted_round_trips_and_is_not_error() {
        // Wire string must match the frontend; interruption is recoverable, so
        // it must not carry the is_error flag (that path drives error handling).
        assert_eq!(TaskTurnSubType::Interrupted.as_str(), "turn_interrupted");
        assert_eq!(TaskTurnSubType::Interrupted.to_string(), "turn_interrupted");
        assert!(!TaskTurnSubType::Interrupted.is_error());
        assert!(!SubType::TaskTurn(TaskTurnSubType::Interrupted).is_error());
    }
}

crate::string_enum!(
    /// Task lifecycle status.
    TaskStatus {
        Open => "open",
        Closed => "closed",
        Error => "error",
    }
);

crate::string_enum!(
    /// Task visibility level.
    Visibility {
        Public => "public",
        Private => "private",
    }
);

crate::string_enum!(
    /// Lifecycle status of a single task turn — the `task_turns.status` column.
    ///
    /// A *turn* is one user-message-and-agent-response cycle. This state is
    /// written transactionally at the gRPC event ingress, so it is authoritative
    /// rather than derived from the event log.
    TurnStatus {
        /// The agent is processing the turn.
        Active => "active",
        /// The agent finished successfully.
        Completed => "completed",
        /// The agent run errored (harness or agent-level).
        Error => "error",
        /// The user stopped the turn.
        Stopped => "stopped",
        /// The pod was interrupted mid-turn (still non-terminal — it resumes on
        /// reschedule, or the reaper reconciles it as a backstop).
        Interrupted => "interrupted",
    }
);

impl TurnStatus {
    /// Parse a `task_turns.status` string. `None` for a value this build does
    /// not know — a row written by a newer backend, which has no honest
    /// counterpart here.
    pub fn parse(s: &str) -> Option<Self> {
        Self::try_from(s).ok()
    }

    /// Whether a turn in this state is still running.
    ///
    /// `Interrupted` counts: the pod went away mid-turn and the run resumes on
    /// the same row. Reading it as ended would show a turn that is about to
    /// carry on as a turn that finished.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Active | Self::Interrupted)
    }

    /// The turn state an event's `(type, sub_type)` reports, or `None` for an
    /// event that does not bear turn state.
    ///
    /// The single table for that mapping. `moatless_tasks` reads it to decide
    /// what an inbound event writes to `task_turns`; the event publisher reads
    /// it to stamp what the event says about a working agent. Two readers of
    /// one table, rather than two tables that have to be kept in step.
    pub fn from_event(event_type: &str, sub_type: &str) -> Option<Self> {
        match (event_type, sub_type) {
            ("taskTurn", "active") => Some(Self::Active),
            ("taskTurn", "turn_interrupted") => Some(Self::Interrupted),
            ("taskTurn" | "agent", "completed") => Some(Self::Completed),
            ("taskTurn" | "agent", "error") => Some(Self::Error),
            ("taskTurn" | "agent", "stopped") => Some(Self::Stopped),
            _ => None,
        }
    }

    /// Whether a turn that ended in this state left the agent session cut off
    /// mid-flight. Resuming such a session replays a transcript whose last
    /// entry is an unanswered tool call, so the next run must start fresh.
    pub fn cut_off_session(self) -> bool {
        matches!(self, Self::Error | Self::Stopped)
    }
}

crate::string_enum!(
    /// Type of a directional link between two tasks. The `from` task points at the
    /// `to` task via this relation (e.g. a child `created_by` its parent).
    ///
    /// `CreatedBy` and `ForkedFrom` are both parent edges and both surface the
    /// `from` task as a child. They differ in what else follows: only
    /// `CreatedBy` closes with its parent and reports its agent errors into the
    /// parent's conversation, because a fork is often the work that continued
    /// and nobody is waiting on its result.
    TaskRelationType {
        CreatedBy => "created_by",
        RelatedTo => "related_to",
        ForkedFrom => "forked_from",
    }
);

/// How a task's checkout relates to other members of the same Sandbox.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CheckoutMode {
    /// One checkout for every member — the single-member behavior too.
    #[default]
    Shared,
    Isolated,
}

impl CheckoutMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Isolated => "isolated",
        }
    }
}

impl std::fmt::Display for CheckoutMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for CheckoutMode {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "shared" => Ok(Self::Shared),
            "isolated" => Ok(Self::Isolated),
            other => Err(format!("unknown CheckoutMode: {other:?}")),
        }
    }
}

impl TryFrom<String> for CheckoutMode {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

#[cfg(test)]
mod wire_contract {
    use super::*;

    /// The wire strings are a contract: `openapi-specs.json` and the generated
    /// TypeScript client both pin them. A changed literal here is a breaking
    /// API change, not a rename.
    #[test]
    fn task_vocabulary_wire_strings() {
        for (value, wire) in [
            (TaskStatus::Open, "open"),
            (TaskStatus::Closed, "closed"),
            (TaskStatus::Error, "error"),
        ] {
            assert_eq!(value.as_str(), wire);
            assert_eq!(TaskStatus::try_from(wire), Ok(value));
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{wire}\"")
            );
        }
        for (value, wire) in [
            (Visibility::Public, "public"),
            (Visibility::Private, "private"),
        ] {
            assert_eq!(value.as_str(), wire);
            assert_eq!(Visibility::try_from(wire), Ok(value));
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{wire}\"")
            );
        }
        for (value, wire) in [
            (CheckoutMode::Shared, "shared"),
            (CheckoutMode::Isolated, "isolated"),
        ] {
            assert_eq!(value.as_str(), wire);
            assert_eq!(CheckoutMode::try_from(wire), Ok(value));
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{wire}\"")
            );
        }
        for (value, wire) in [
            (TaskRelationType::CreatedBy, "created_by"),
            (TaskRelationType::RelatedTo, "related_to"),
            (TaskRelationType::ForkedFrom, "forked_from"),
        ] {
            assert_eq!(value.as_str(), wire);
            assert_eq!(TaskRelationType::try_from(wire), Ok(value));
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{wire}\"")
            );
        }
        assert_eq!(CheckoutMode::default(), CheckoutMode::Shared);
    }
}

// =============================================================================
// Task
// =============================================================================

/// A unit of agent work.
///
/// The persisted row is wider than this: `tasks.created_at` is a text column,
/// `created_at_db` is a second timestamp the domain has no use for, and the
/// enum columns are text. Reading those back is `moatless_tasks`' job — see
/// `task::models::TaskRow`.
#[derive(Debug, Clone)]
pub struct Task {
    pub task_id: String,
    pub short_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_message_at: DateTime<Utc>,
    pub agent_type: AgentType,
    pub repository_id: String,
    /// Workspace this task runs against (Phase A: nullable during the additive
    /// transition; backfilled to the repository's shadow workspace). When set,
    /// provisioning resolves the repository composition via the workspace;
    /// otherwise it falls back to `repository_id`.
    pub workspace_id: Option<String>,
    /// Sandbox this task belongs to. A task with no id here has never been
    /// started; the ordinary first start mints a sandbox whose id equals the
    /// task id.
    pub sandbox_id: Option<String>,
    /// Checkout relationship to other members of the Sandbox. `None` for tasks
    /// predating shared-sandbox membership, which read as shared.
    pub checkout_mode: Option<CheckoutMode>,
    pub status: TaskStatus,
    pub total_cost_usd: f64,
    pub site_url: Option<String>,
    pub template_id: Option<String>,
    pub vibe: Option<bool>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub site_id: Option<String>,
    pub site_provider: Option<String>,
    pub branch: Option<String>,
    /// Current requested model for this task. Updated when a user message
    /// explicitly names a model and reused for later messages that omit one.
    pub model: Option<String>,
    pub created_by: Option<String>,
    pub visibility: Visibility,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    pub append_system_prompt: Option<String>,
    pub headless: bool,
    /// Idle TTL in minutes. The reaper stops the sandbox when no activity has
    /// occurred for this many minutes. `0` means "indefinite — never reap".
    pub ttl_minutes: i32,
    /// Inactive TTL in days. The reaper auto-closes the task when no activity
    /// has occurred for this many days. `0` means "indefinite — never close".
    pub inactive_ttl_days: i32,
    /// Loop that spawned this task (subscription + schedule). Independent from
    /// any communication binding — schedule loops never create a binding.
    pub loop_id: Option<String>,
    /// Skills every fresh agent session on this task activates. Set by the Loop
    /// that spawned it; re-frontloaded after a restart or a start-fresh send,
    /// because the transcript that carried them is gone.
    pub skills: Vec<String>,
}

impl Task {
    /// The task as a [`ScopedResource`] for access-control checks
    /// (`ScopeContext::can_read` / `can_write`). Ownership falls back to
    /// `created_by` for rows predating the `owner_user_id` column, matching the
    /// `COALESCE` in the DAO's scope filter.
    pub fn scoped_resource(&self) -> ScopedResource {
        ScopedResource::borrowed(
            self.scope,
            self.owner_user_id.as_deref().or(self.created_by.as_deref()),
        )
    }
}
