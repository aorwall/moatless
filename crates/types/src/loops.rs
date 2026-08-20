//! Loop vocabulary: how a Loop routes external messages onto Tasks, and
//! the context an Adapter is invoked with.
//!
//! The module is plural because `loop` is a Rust keyword.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::access_control::Scope;
use crate::adapter::AdapterKind;
use crate::agent::AgentType;

/// Loop routing mode for message-to-task mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    Ongoing,
    BySubject,
}

impl RoutingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ongoing => "ongoing",
            Self::BySubject => "by_subject",
        }
    }
}

impl std::fmt::Display for RoutingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for RoutingMode {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl TryFrom<&str> for RoutingMode {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "ongoing" => Ok(Self::Ongoing),
            "by_subject" => Ok(Self::BySubject),
            other => Err(format!("unknown RoutingMode: {other:?}")),
        }
    }
}

/// Adapter-facing Loop context.
///
/// This is synthesized from canonical Loops, Subscriptions, Bindings, or
/// schedule firings before invoking Adapter-specific delivery/parsing code.
#[derive(Debug, Clone)]
pub struct LoopContext {
    pub id: String,
    /// The Adapter this Loop listens on, or `"scheduled"` for a Loop a cron
    /// rule fires. Wider than [`AdapterKind`] because a schedule-fired Loop has
    /// no Adapter behind it.
    pub adapter_kind: String,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    pub active: bool,
    pub allow_direct_send: bool,
    pub adapter_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub scope: Scope,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    /// Skills every task this Loop spawns activates on each fresh agent session.
    /// Names only; the sandbox resolves each one's `SKILL.md` on disk.
    pub skills: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: AgentType,
    /// Per-Loop inactivity TTL override (days). `None` means the reaper
    /// falls back to the platform-wide default (`TASK_INACTIVE_TIMEOUT_DAYS`).
    /// Flows into `tasks.inactive_ttl_days` at task creation time.
    pub inactive_ttl_days: Option<i32>,
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

/// The `loop_configs` row a Loop spawns Tasks from.
#[derive(Debug, Clone)]
pub struct LoopSpawnConfig {
    pub id: String,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: AgentType,
    pub inactive_ttl_days: Option<i32>,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl LoopSpawnConfig {
    /// The administrator-approved execution identity.
    // Runnable spawn-config queries exclude rows without this value.
    #[allow(clippy::expect_used)]
    pub fn effective_user_id(&self) -> &str {
        self.run_as_user_id
            .as_deref()
            .expect("runnable Loop must have an explicit run-as user")
    }
}

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
    pub repository_id: String,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: AgentType,
    pub inactive_ttl_days: Option<i32>,
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

/// A cron rule that fires a Loop on a timer.
#[derive(Debug, Clone)]
pub struct Schedule {
    pub id: String,
    pub loop_id: String,
    pub cron_expression: String,
    pub timezone: String,
    pub message_template: String,
    pub run_as_user_id: Option<String>,
    pub active: bool,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub next_fire_at: DateTime<Utc>,
    pub locked_by: Option<String>,
    pub locked_until: Option<DateTime<Utc>>,
    pub created_by: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
