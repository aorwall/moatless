//! Loop vocabulary: how a Loop routes external messages onto Tasks, and the
//! context an Adapter is invoked with.
//!
//! The module is plural because `loop` is a Rust keyword.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::access_control::Scope;
use crate::adapter::AdapterKind;
use crate::agent::AgentType;

crate::string_enum!(
    /// Loop routing mode for message-to-task mapping.
    RoutingMode {
        Ongoing => "ongoing",
        BySubject => "by_subject",
    }
);

/// Adapter-facing Loop context.
///
/// Synthesized from canonical Loops, Subscriptions, Bindings, or schedule
/// firings before invoking Adapter-specific delivery/parsing code.
#[derive(Debug, Clone)]
pub struct LoopContext {
    pub id: String,
    /// The Adapter this Loop listens on, or
    /// [`SCHEDULED_ORIGIN`](crate::adapter::SCHEDULED_ORIGIN) for a cron-fired
    /// Loop.
    ///
    /// Wider than [`AdapterKind`], since a schedule-fired Loop has no Adapter
    /// behind it.
    pub adapter_kind: String,
    pub source_id: String,
    pub source_name: Option<String>,
    /// The Workspace every task this Loop spawns runs in, and what the task's
    /// repositories are resolved from — its primary placement is what the task
    /// runs against.
    pub workspace_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    pub active: bool,
    pub allow_direct_send: bool,
    pub adapter_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub scope: Scope,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    /// Skills every task this Loop spawns activates on each fresh agent
    /// session. Names only; the sandbox resolves each one's `SKILL.md` on disk.
    pub skills: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: AgentType,
    /// Per-Loop model override. `None` keeps the platform default the sandbox
    /// resolves for `agent_type`. Flows into `tasks.model` at task creation.
    pub model: Option<String>,
    /// Per-Loop inactivity TTL override (days). `None` means the reaper falls
    /// back to the platform-wide default (`TASK_INACTIVE_TIMEOUT_DAYS`). Flows
    /// into `tasks.inactive_ttl_days` at task creation time.
    pub inactive_ttl_days: Option<i32>,
    /// Extra node labels for the sandboxes this Loop spawns, merged over the
    /// deployment-wide selector. Outranks the Workspace's own value, and `None`
    /// falls back to it.
    pub node_selector: Option<BTreeMap<String, String>>,
    /// Stop this Loop's sandbox as soon as its agent run finishes, rather than
    /// holding the pod for the idle TTL. Keeps a long autonomous run on
    /// dedicated capacity from outliving the work it was started for.
    pub stop_when_done: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl LoopContext {
    /// The administrator-approved execution identity.
    ///
    /// Runnable Loop queries exclude rows without this value.
    // The database query establishes this invariant before constructing a
    // runnable Loop context.
    #[allow(clippy::expect_used)]
    pub fn effective_user_id(&self) -> &str {
        self.run_as_user_id
            .as_deref()
            .expect("runnable Loop must have an explicit run-as user")
    }
}

crate::string_enum!(
    /// Kind of firing source owned by a Loop.
    LoopKind {
        AdapterEvent => "adapter_event",
        Schedule => "schedule",
        Manual => "manual",
    }
);

crate::string_enum!(
    /// Authority from which the current Loop configuration was loaded.
    LoopConfigSource {
        Git => "git",
        Manual => "manual",
    }
);

// =============================================================================
// Loop spawn configuration
// =============================================================================

/// A Subscription resolved against its Loop and spawn configuration — what the
/// router matches an inbound event on.
#[derive(Debug, Clone)]
pub struct SubscriptionMatch {
    pub id: String,
    /// The Loop's own name (`loops.name`, NOT NULL since migration 0115).
    pub name: String,
    pub adapter_kind: AdapterKind,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub routing_mode: RoutingMode,
    pub event_filter: serde_json::Value,
    /// See [`LoopContext::workspace_id`].
    pub workspace_id: String,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: AgentType,
    /// Per-Loop model override. See [`LoopContext::model`].
    pub model: Option<String>,
    pub inactive_ttl_days: Option<i32>,
    /// See [`LoopContext::node_selector`].
    pub node_selector: Option<BTreeMap<String, String>>,
    /// See [`LoopContext::stop_when_done`]. `None` is the Loop declaring
    /// nothing, which resolves to `false`.
    pub stop_when_done: Option<bool>,
    /// Seconds inbound events for one Binding are held so those arriving
    /// together reach the Task as a single message. `None` is the Loop
    /// declaring nothing, which falls back to the per-adapter environment
    /// default; `0` delivers every event on its own.
    pub message_buffer_seconds: Option<i32>,
    pub allow_direct_send: bool,
    pub adapter_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
    pub active: bool,
    pub created_by: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SubscriptionMatch {
    /// The administrator-approved execution identity.
    // Runnable subscription queries exclude rows without this value.
    #[allow(clippy::expect_used)]
    pub fn effective_user_id(&self) -> &str {
        self.run_as_user_id
            .as_deref()
            .expect("runnable Loop must have an explicit run-as user")
    }
}
