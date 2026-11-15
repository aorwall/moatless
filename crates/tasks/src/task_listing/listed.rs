//! The row a Task listing returns, and the values it aggregates.

use chrono::{DateTime, Utc};
use moatless_types::agent::{AgentProfile, Harness};
use moatless_types::sandbox::{SandboxDesiredState, SandboxStopReason};
use moatless_types::task::Visibility;
use serde::{Deserialize, Serialize};
use sqlx::types::Json;

/// One Task, with the derived values a sidebar row is assembled from.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ListedTask {
    pub task_id: String,
    /// The Workspace this Task runs against. `None` for older and hand-written
    /// rows: creation makes a shadow Workspace when none is given, but
    /// provisioning still falls back to repository-only behaviour without one.
    pub workspace_id: Option<String>,
    pub name: Option<String>,
    pub branch: Option<String>,
    #[sqlx(try_from = "String")]
    pub agent_profile: ListedAgent,
    /// Creation time as the caller recorded it (RFC 3339 text, `NOT NULL`).
    pub created_at: String,
    /// Creation time as the database recorded it. `None` for rows written
    /// before the column existed.
    pub created_at_db: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    /// When this Task was closed. `None` while it is open, which is every row
    /// the live listing returns.
    pub closed_at: Option<DateTime<Utc>>,
    /// Who may read this Task. `NOT NULL`, defaulting to `Private`.
    #[sqlx(try_from = "String")]
    pub visibility: Visibility,
    /// The owner's display name, `None` when the viewer owns the Task or the
    /// owner resolves to no user — a per-viewer value, like the follower state
    /// below. Owner is the access rule's own `COALESCE(owner_user_id,
    /// created_by)`.
    pub owner_name: Option<String>,

    /// The newest Turn, by turn number. `None` when the Task has never run.
    pub turn_number: Option<i32>,
    pub turn_status: Option<String>,
    pub turn_started_at: Option<DateTime<Utc>>,
    pub turn_ended_at: Option<DateTime<Utc>>,
    /// The error text the newest Turn ended with: `None` unless its status is
    /// `error`, so the next Turn starting or completing clears it.
    pub turn_error: Option<String>,
    /// The `agent.error` reason that error carried (`usage_limit`, …).
    pub turn_error_reason: Option<String>,
    /// When a usage limit lifts (RFC 3339 text), as the provider stated it.
    pub turn_error_resets_at: Option<String>,

    /// Durable Sandbox lifecycle intent; all four are `None` together when no
    /// Sandbox was ever requested — the lazy-absence model of `sandboxes`.
    ///
    /// Persisted column; read the typed value through
    /// [`ListedTask::sandbox_desired_state`].
    pub sandbox_desired_state_raw: Option<String>,
    /// Persisted column; read the typed value through
    /// [`ListedTask::sandbox_stop_reason`].
    pub sandbox_stop_reason_raw: Option<String>,
    pub sandbox_started_at: Option<DateTime<Utc>>,
    pub sandbox_updated_at: Option<DateTime<Utc>>,
    /// `moat cmd` commands the Sandbox is running, as its pod last reported.
    /// `0` for a Task with no Sandbox row, which can be running nothing.
    pub running_commands: i32,
    /// The labels of those commands, oldest first. Mirrored by a separate
    /// write, so it can briefly hold fewer than `running_commands`.
    #[sqlx(json)]
    pub running_command_labels: Vec<String>,
    /// See `listed_dao::listed_children_join`.
    pub child_count: i64,
    #[sqlx(json)]
    pub running_children: Vec<RunningChild>,

    /// When a person last wrote: the newest `user` row that is not a sandbox
    /// wake (inbox `origin = 'sandbox'`); the agent's tool results are `system`.
    ///
    /// Not `tasks.last_message_at`: that also advances when the agent
    /// finishes, stops, or errors.
    pub latest_user_message_at: Option<DateTime<Utc>>,

    /// The newest inbox item including sandbox wakes: what running state
    /// reads, since a queued wake owes a turn as much as a person's message.
    pub latest_queued_message_at: Option<DateTime<Utc>>,

    /// The model the newest Turn's agent replies reported they were produced
    /// by, newest first. `None` until something comes back.
    pub assistant_model: Option<String>,
    /// The model asked for when the newest Turn started, or — when it has not
    /// started — the choice carried by the messages queued for it.
    pub requested_model: Option<String>,

    /// An `AskUserQuestion` raised by the agent that nothing has answered.
    pub pending_question: bool,
    /// An `ExitPlanMode` plan proposed by the agent that nothing has answered.
    pub pending_plan: bool,

    /// The Task this one was spawned by (`created_by`) or forked from
    /// (`forked_from`), when the viewer may read it; all three are `None`
    /// otherwise. `fork_at_turn` is the fork edge's `at_turn`.
    pub parent_task_id: Option<String>,
    pub parent_relation: Option<String>,
    pub fork_at_turn: Option<i32>,
    pub side_chat: bool,

    pub settled_override: Option<String>,
    pub settled_at: Option<DateTime<Utc>>,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub snoozed_at: Option<DateTime<Utc>>,
    /// How far the viewer has read this Task (`task_read_receipts`), or
    /// `None` when they never opened it.
    pub last_read_at: Option<DateTime<Utc>>,

    /// When the viewer pinned this Task; `None` while it is not pinned.
    pub pinned_at: Option<DateTime<Utc>>,
    /// The viewer's fractional-index slots in the pinned and active blocks.
    pub pin_order_key: Option<String>,
    pub active_order_key: Option<String>,
    /// When the viewer opted this Task out of auto-settle; `None` while the
    /// reaper may settle it for them.
    pub auto_settle_disabled_at: Option<DateTime<Utc>>,

    /// Every GitHub pull request this Task is bound to, primary first.
    ///
    /// Empty, not `None`, when there's no active binding. Ordered live before
    /// merged or closed, then newest number: t3code's `primary_pull_request`
    /// restates that order, and the two must agree on which is primary.
    pub pull_requests: Option<Json<Vec<ListedPullRequest>>>,

    /// The Task's tag names, sorted; empty when it has none.
    #[sqlx(json)]
    pub tags: Vec<String>,
}

impl ListedTask {
    /// Persisted intent for this Task's Sandbox.
    pub fn sandbox_desired_state(&self) -> Option<SandboxDesiredState> {
        let raw = self.sandbox_desired_state_raw.as_deref()?;
        match SandboxDesiredState::try_from(raw) {
            Ok(state) => Some(state),
            Err(error) => {
                tracing::warn!(task_id = %self.task_id, %error, "unreadable desired_state column");
                None
            }
        }
    }

    /// Why the Sandbox was moved away from running, when it was.
    pub fn sandbox_stop_reason(&self) -> Option<SandboxStopReason> {
        let raw = self.sandbox_stop_reason_raw.as_deref()?;
        match SandboxStopReason::try_from(raw) {
            Ok(reason) => Some(reason),
            Err(error) => {
                tracing::warn!(task_id = %self.task_id, %error, "unreadable stop_reason column");
                None
            }
        }
    }
}

/// A spawned child Task still at work, as `listed_dao::listed_children_join` lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunningChild {
    pub task_id: String,
    pub name: Option<String>,
}

/// One GitHub pull request bound to a Task, as the listing query aggregates it.
///
/// `subject_key` alone makes a row reportable, linking from the moment it's
/// written; `title`, `base_ref` and `head_ref` come from a refresh's status
/// blob and are `None` together until one runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListedPullRequest {
    pub repository: String,
    pub number: i64,
    pub url: String,
    pub title: Option<String>,
    pub base_ref: Option<String>,
    pub head_ref: Option<String>,
    /// GitHub's own last change to the pull request, as the blob stores it.
    /// `None` until a refresh or webhook writes one.
    #[serde(default)]
    pub updated_at: Option<String>,
    /// The head commit's checks rollup, T3's spelling (`passing`, `failing`,
    /// `pending`). `None` with no checks, or until a live read rolls them up.
    #[serde(default)]
    pub checks_state: Option<String>,
    /// The binding's normalized state kind, in its own column rather than the
    /// blob: `open`, `draft`, `merged` or `closed`, and `None` until the
    /// adapter has classified one.
    pub state_kind: Option<String>,
    /// When this deployment last read the pull request's state from GitHub,
    /// falling back to the binding's `NOT NULL` `last_event_at` so it resolves
    /// for an unrefreshed binding too.
    pub status_read_at: DateTime<Utc>,
    /// When the binding was written. T3's `linkedAt`, and what orders links of
    /// equal standing there.
    pub linked_at: DateTime<Utc>,
    /// `task_bindings.origin`.
    pub origin: String,
    /// `task_bindings.metadata.bind_reason`; `None` for a Binding that records
    /// none, and for a blob written before the key was read.
    #[serde(default)]
    pub bind_reason: Option<String>,
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

/// The agent a [`ListedTask`] runs, as stored.
///
/// `Unknown` keeps an id this build does not know, such as one a newer backend
/// wrote during a rollout: the row still lists, and names no agent it is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListedAgent {
    Known(AgentProfile),
    Unknown(String),
}

impl ListedAgent {
    /// The stored `agent_profile` id.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Known(profile) => profile.as_str(),
            Self::Unknown(id) => id,
        }
    }

    /// The driver family behind the agent; `None` when this build does not
    /// know the agent.
    pub fn harness(&self) -> Option<Harness> {
        match self {
            Self::Known(profile) => Some(profile.harness()),
            Self::Unknown(_) => None,
        }
    }
}

impl From<AgentProfile> for ListedAgent {
    fn from(profile: AgentProfile) -> Self {
        Self::Known(profile)
    }
}

impl From<String> for ListedAgent {
    fn from(id: String) -> Self {
        AgentProfile::parse(&id).map_or(Self::Unknown(id), Self::Known)
    }
}

impl std::fmt::Display for ListedAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where an archive page resumes: the sort key of the last row the page
/// before it returned, in the listing's `updated_at DESC NULLS LAST, task_id`
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingCursor {
    pub(crate) updated_at: Option<DateTime<Utc>>,
    pub(crate) task_id: String,
}

impl ListingCursor {
    /// The cursor resuming after `row`.
    pub fn after(row: &ListedTask) -> Self {
        Self {
            updated_at: row.updated_at,
            task_id: row.task_id.clone(),
        }
    }

    /// The opaque text a client echoes back: `<updated_at>|<task_id>`, the
    /// time empty when absent. Microsecond precision, since that is what
    /// Postgres stores and a coarser time would skip or repeat rows.
    pub fn encode(&self) -> String {
        let updated_at = self
            .updated_at
            .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
            .unwrap_or_default();
        format!("{updated_at}|{}", self.task_id)
    }

    /// `None` for text [`Self::encode`] did not produce.
    pub fn decode(text: &str) -> Option<Self> {
        let (updated_at, task_id) = text.split_once('|')?;
        if task_id.is_empty() {
            return None;
        }
        let updated_at = match updated_at {
            "" => None,
            at => Some(DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&Utc)),
        };
        Some(Self {
            updated_at,
            task_id: task_id.to_owned(),
        })
    }
}
