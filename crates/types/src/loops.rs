//! Loop vocabulary: how a Loop routes external messages onto Tasks, and the
//! context an Adapter is invoked with.
//!
//! The module is plural because `loop` is a Rust keyword.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::access_control::Scope;
use crate::adapter::AdapterKind;
use crate::agent::AgentProfile;

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
    /// See [`crate::task::Task::memory_preload`].
    pub memory_preload: Vec<String>,
    /// See [`crate::task::Task::agent_properties`].
    pub agent_properties: crate::agent::AgentProperties,
    pub run_as_user_id: Option<String>,
    pub agent_profile: AgentProfile,
    /// Per-Loop model override. `None` keeps the platform default the sandbox
    /// resolves for `agent_profile`. Flows into `tasks.model` at task creation.
    pub model: Option<String>,
    /// Per-Loop days from settling to auto-closing. `None` falls back to the
    /// platform-wide default (`TASK_CLOSE_AFTER_SETTLED_DAYS`). Flows into
    /// `tasks.close_after_settled_days` at task creation time.
    pub close_after_settled_days: Option<i32>,
    /// Per-Loop idle hours before the reaper settles a spawned task. `None`
    /// falls back to `DEFAULT_LOOP_SETTLE_AFTER_HOURS`. Flows into
    /// `tasks.settle_after_hours` at task creation time.
    pub settle_after_hours: Option<i32>,
    /// Extra node labels for the sandboxes this Loop spawns, merged over the
    /// deployment-wide selector. Outranks the Workspace's own value, and `None`
    /// falls back to it.
    pub node_selector: Option<BTreeMap<String, String>>,
    /// Stop this Loop's sandbox as soon as its agent run finishes, rather than
    /// holding the pod for the idle TTL. Keeps a long autonomous run on
    /// dedicated capacity from outliving the work it was started for.
    pub stop_when_done: bool,
    /// The Task Room every task this Loop spawns gets. `None` opens none.
    pub room: Option<RoomSpec>,
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

/// Where a Loop's Tasks get their Task Room: a conversation space an Adapter
/// opens for one Task, bound to it for its life. `LOOP.md`'s `room` key.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomSpec {
    /// The Adapter that hosts the room.
    pub via: AdapterKind,
    /// The `via` connection to open it on. `None` takes the triggering event's
    /// connection, else the only one `via` has.
    #[serde(
        default,
        rename = "connection",
        skip_serializing_if = "Option::is_none"
    )]
    pub connection_id: Option<String>,
    /// Where the room is announced, in `via`'s own destination syntax (what
    /// `moat send --to` takes). `None` anchors on the triggering event, which
    /// must then have come through `via`.
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// Who to add when the room opens: `requester` (who the Task was opened
    /// for), `run_as` (the Loop's identity), or a Moatless login.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invite: Vec<String>,
    /// Force the room's privacy. `None` lets `via` follow its anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<bool>,
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
    /// See [`crate::task::Task::memory_preload`].
    pub memory_preload: Vec<String>,
    /// See [`crate::task::Task::agent_properties`].
    pub agent_properties: crate::agent::AgentProperties,
    pub run_as_user_id: Option<String>,
    pub agent_profile: AgentProfile,
    /// Per-Loop model override. See [`LoopContext::model`].
    pub model: Option<String>,
    pub close_after_settled_days: Option<i32>,
    pub settle_after_hours: Option<i32>,
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
    /// Quiet period in seconds: a burst flushes once this long passes after
    /// its newest event, never later than `message_buffer_seconds` allows.
    /// `None` is the Loop declaring nothing, so `QUIET_PERIOD` applies.
    pub message_quiet_seconds: Option<i32>,
    /// See [`LoopContext::room`].
    pub room: Option<RoomSpec>,
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

/// Id prefix of a Subscription derived from a connection rather than read from
/// `loops`. Nothing persists such a row, so a reader that goes looking for one
/// by id finds nothing.
const DELIVERY_ONLY_PREFIX: &str = "virtual:";

impl SubscriptionMatch {
    /// The id a Subscription built from `connection_id` alone carries.
    pub fn delivery_only_id(connection_id: &str) -> String {
        format!("{DELIVERY_ONLY_PREFIX}{connection_id}")
    }

    /// Whether this Subscription was derived from a connection with no Loop.
    /// Never store this id where a Loop's own id is expected.
    pub fn is_delivery_only(&self) -> bool {
        Self::is_delivery_only_id(&self.id)
    }

    /// Whether `id` is one [`Self::delivery_only_id`] built.
    pub fn is_delivery_only_id(id: &str) -> bool {
        id.starts_with(DELIVERY_ONLY_PREFIX)
    }

    /// The administrator-approved execution identity.
    // Runnable subscription queries exclude rows without this value.
    #[allow(clippy::expect_used)]
    pub fn effective_user_id(&self) -> &str {
        self.run_as_user_id
            .as_deref()
            .expect("runnable Loop must have an explicit run-as user")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_reads_its_loop_md_keys() {
        let spec: RoomSpec = serde_json::from_value(serde_json::json!({
            "via": "slack",
            "in": "channel:C0FACTORY",
            "invite": ["requester", "run_as", "ana"],
            "private": true,
        }))
        .unwrap();

        assert_eq!(
            spec,
            RoomSpec {
                via: AdapterKind::Slack,
                connection_id: None,
                anchor: Some("channel:C0FACTORY".into()),
                invite: vec!["requester".into(), "run_as".into(), "ana".into()],
                private: Some(true),
            }
        );
        let stored = serde_json::to_value(&spec).unwrap();
        assert_eq!(serde_json::from_value::<RoomSpec>(stored).unwrap(), spec);
    }
}
