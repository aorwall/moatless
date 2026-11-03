//! Task vocabulary: the Task itself, the provenance a Message carries into one,
//! and what a Task's and a Turn's events say.

// reason: test code may unwrap freely; production code keeps the workspace `deny`.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::access_control::{Scope, ScopedResource};
use crate::agent::AgentProfile;

/// The provenance that says a Message reached this Task from outside it.
///
/// Carried as one value rather than six positional arguments; an internal send
/// (a person in the composer, task creation) passes [`ExternalOrigin::none`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ExternalOrigin<'a> {
    pub event_id: Option<&'a str>,
    pub adapter: Option<&'a str>,
    pub subject_key: Option<&'a str>,
    pub source_name: Option<&'a str>,
    /// The handle the external system knows the writer by, recorded verbatim
    /// and never resolved to a Moatless identity.
    pub author: Option<&'a str>,
    /// The writer's own id in the external system — a Slack `U0000000001`. What
    /// the sender label resolves through, since the Moatless user id on the
    /// send is the subscription's owner when the writer is unmapped.
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
/// relation immediately after the new task is inserted.
///
/// Use when a webhook spawned the new task and discovered a peer task sharing
/// the same external artifact (binding `subject_key`) — the peer is not a
/// parent.
#[derive(Debug, Clone)]
pub struct RelatedTask {
    pub related_task_id: String,
    pub context: serde_json::Value,
}

crate::string_enum!(
    /// Task sub-types.
    TaskSubType [Hash] {
        Created => "created",
        Open => "open",
        Closed => "closed",
        Updated => "updated",
        MetadataUpdated => "metadata.updated",
        /// A backend failure a person should see in the chat, e.g. a failed
        /// wake; projected as an Error Message.
        Error => "error",
        /// A message sent to a Binding's channel: the agent's reply, which the
        /// conversation shows, or a stub announcing a run, which it does not.
        BindingReplySent => "binding_reply_sent",
        /// The platform moved the Task to another agent, or back; projected as
        /// an activity whose payload `summary` says why.
        AgentSwitched => "agent_switched",
        /// An Artifact version published to be shown inline in the thread;
        /// projected as an `ArtifactShown` activity.
        ArtifactShown => "artifact_shown",
        /// An agent asked for a secret; projected as a `SecretRequested`
        /// activity.
        SecretRequested => "secret_requested",
        /// A person answered it; projected as a `SecretRequestAnswered`
        /// activity.
        SecretRequestAnswered => "secret_request_answered",
    }
);

crate::string_enum!(
    /// `TaskTurn` sub-types.
    TaskTurnSubType [Hash] {
        Active => "active",
        Completed => "completed",
        Error => "error",
        Stopped => "stopped",
        /// The turn's sandbox pod terminated involuntarily (eviction, node
        /// loss) mid-turn. Emitted immediately by the pod watcher, ahead of
        /// the delayed orphaned-turn reconciliation. Resumes once the pod
        /// reschedules — not a terminal error, so `is_error` stays false.
        Interrupted => "turn_interrupted",
    }
);

impl TaskTurnSubType {
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error)
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

/// `TaskTurn` event payload — typed shape for `EventType::TaskTurn` events.
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
    /// A *turn* is one user-message-and-agent-response cycle; this state is
    /// written transactionally at the gRPC event ingress, so it's authoritative
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
    /// the same row; reading it as ended would show a turn about to carry on as
    /// one that finished.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Active | Self::Interrupted)
    }

    /// The turn state an event's `(type, sub_type)` reports, or `None` for an
    /// event that does not bear turn state.
    ///
    /// The single table for that mapping: `moatless_tasks` uses it to decide
    /// what an inbound event writes to `task_turns`; the event publisher
    /// reads it too — two readers of one table, not two tables kept in step.
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
    /// Type of a directional link between two tasks: the `from` task points at
    /// the `to` task via this relation (e.g. a child `created_by` its parent).
    ///
    /// Both `CreatedBy` and `ForkedFrom` surface `from` as a child, but only
    /// `CreatedBy` closes with its parent and notices it of each Turn's
    /// completion or error: nobody waits on a fork.
    TaskRelationType {
        CreatedBy => "created_by",
        RelatedTo => "related_to",
        ForkedFrom => "forked_from",
    }
);

impl TaskRelationType {
    /// The relation types that make the `from` Task a child of the `to` Task.
    ///
    /// `RelatedTo` is deliberately excluded: a cross-referenced Task is not work
    /// the `to` Task spawned.
    pub const PARENT: [Self; 2] = [Self::CreatedBy, Self::ForkedFrom];

    /// [`Self::PARENT`]'s wire values, for binding as a `text[]`.
    pub const PARENT_WIRE: [&'static str; 2] = [Self::PARENT[0].as_str(), Self::PARENT[1].as_str()];

    /// [`Self::PARENT`] as a SQL `IN` list, for a fragment embedded in a
    /// statement whose parameter slots it does not own.
    pub const PARENT_SQL_LIST: &'static str = "('created_by', 'forked_from')";

    /// Whether this relation makes the `from` Task a child of the `to` Task.
    #[must_use]
    pub const fn is_parent(self) -> bool {
        matches!(self, Self::CreatedBy | Self::ForkedFrom)
    }
}

crate::string_enum!(
    /// How a repository came to be one a Task works in.
    TaskRepositorySource {
        /// Placed in the Task's Workspace when the Task was created.
        Workspace => "workspace",
        /// Cloned from inside the Task's sandbox.
        Cloned => "cloned",
    }
);

crate::string_enum!(
    /// How a task's checkout relates to other members of the same Sandbox.
    CheckoutMode [Default] {
        /// One checkout for every member — the single-member behavior too.
        #[default]
        Shared => "shared",
        Isolated => "isolated",
    }
);

#[cfg(test)]
mod wire_contract {
    use super::*;

    #[test]
    fn the_parent_rule_is_stated_the_same_way_in_every_form() {
        for relation in TaskRelationType::ALL {
            assert_eq!(
                relation.is_parent(),
                TaskRelationType::PARENT.contains(relation),
                "{relation}"
            );
        }
        let quoted: Vec<String> = TaskRelationType::PARENT_WIRE
            .iter()
            .map(|wire| format!("'{wire}'"))
            .collect();
        assert_eq!(
            TaskRelationType::PARENT_SQL_LIST,
            format!("({})", quoted.join(", "))
        );
    }

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
        for (value, wire) in [
            (TaskRepositorySource::Workspace, "workspace"),
            (TaskRepositorySource::Cloned, "cloned"),
        ] {
            assert_eq!(value.as_str(), wire);
            assert_eq!(TaskRepositorySource::try_from(wire), Ok(value));
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
/// The persisted row is wider than this: `created_at` is text, `created_at_db`
/// is a second timestamp the domain doesn't use, and enum columns are text.
/// Reading those back is `moatless_tasks`' job — see `task::models::TaskRow`.
#[derive(Debug, Clone)]
pub struct Task {
    pub task_id: String,
    pub short_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_message_at: DateTime<Utc>,
    pub agent_profile: AgentProfile,
    /// The Workspace's primary repository when the task was created. `None`
    /// for a task on a Workspace that places no repository.
    pub repository_id: Option<String>,
    /// Workspace this task runs against, which provisioning resolves the
    /// repository composition through.
    ///
    /// Every created task names one; nullable only for rows written before
    /// Workspaces existed, which fall back to `repository_id`.
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
    /// The verified person who requested a Loop's work; independent of following.
    pub reporter_user_id: Option<String>,
    pub visibility: Visibility,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    /// The identity this task's Sandbox acts as outward (git-host credentials,
    /// delivered plugins), when the task names one itself.
    ///
    /// `None` leaves the Workspace's account in charge, falling back to the
    /// owner, per [`crate::access_control::Actor::run_as`].
    pub run_as_user_id: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    pub append_system_prompt: Option<String>,
    pub headless: bool,
    /// Idle TTL in minutes. The reaper stops the sandbox when no activity has
    /// occurred for this many minutes. `0` means "indefinite — never reap".
    pub ttl_minutes: i32,
    /// Days the reaper waits after the task is settled before closing it.
    /// `0` means "indefinite — never close".
    pub close_after_settled_days: i32,
    /// Idle hours after which the reaper settles the task. `None` follows the
    /// deployment default (`TASK_SETTLE_AFTER_HOURS`); `0` never settles.
    pub settle_after_hours: Option<i32>,
    /// Loop that spawned this task (subscription + schedule). Independent from
    /// any communication binding — schedule loops never create a binding.
    pub loop_id: Option<String>,
    /// Skills every fresh agent session on this task activates. Set by the Loop
    /// that spawned it; re-frontloaded after a restart or a start-fresh send,
    /// because the transcript that carried them is gone.
    pub skills: Vec<String>,
    /// Memory keys, or `prefix*` patterns, whose summaries this task's system
    /// prompt lists. Set by the Loop that spawned it; empty preloads nothing.
    pub memory_preload: Vec<String>,
}

impl Task {
    /// The task as a [`ScopedResource`] for access-control checks
    /// (`ScopeContext::can_read` / `can_write`).
    ///
    /// Ownership falls back to `created_by` for rows predating `owner_user_id`,
    /// matching the `COALESCE` in the DAO's scope filter.
    pub fn scoped_resource(&self) -> ScopedResource {
        ScopedResource::borrowed(
            self.scope,
            self.owner_user_id.as_deref().or(self.created_by.as_deref()),
        )
    }
}
