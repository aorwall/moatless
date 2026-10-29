//! Loop-owned execution configuration, schedule state, and the values
//! [`crate::loops::operations::Loops`] reads and writes a Loop through.
use chrono::{DateTime, Utc};
use moatless_types::{
    access_control::Scope,
    adapter::AdapterKind,
    agent::AgentProfile,
    loops::{LoopConfigSource, LoopKind, RoomSpec, RoutingMode},
};

/// The `loop_configs` row a Loop spawns Tasks from.
#[derive(Debug, Clone)]
pub struct LoopSpawnConfig {
    pub id: String,
    /// See [`moatless_types::loops::LoopContext::workspace_id`].
    pub workspace_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub memory_preload: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_profile: AgentProfile,
    /// Per-Loop model override. See
    /// [`moatless_types::loops::LoopContext::model`].
    pub model: Option<String>,
    pub close_after_settled_days: Option<i32>,
    pub settle_after_hours: Option<i32>,
    /// See [`moatless_types::loops::LoopContext::node_selector`].
    pub node_selector: Option<std::collections::BTreeMap<String, String>>,
    /// See [`moatless_types::loops::LoopContext::stop_when_done`].
    pub stop_when_done: Option<bool>,
    /// See [`moatless_types::loops::SubscriptionMatch::message_buffer_seconds`].
    pub message_buffer_seconds: Option<i32>,
    /// See [`moatless_types::loops::LoopContext::room`].
    pub room: Option<RoomSpec>,
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

// =============================================================================
// Loop rows — the read shape `LoopDao` decodes into and the HTTP surface
// projects from. Crate-private: no caller outside this crate reads a row.
// =============================================================================

#[derive(Debug, Clone)]
pub(crate) struct LoopRow {
    pub id: String,
    pub name: String,
    pub kind: LoopKind,
    pub active: bool,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub config_source: LoopConfigSource,
    pub source_repository_id: Option<String>,
    pub source_key: Option<String>,
    pub source_config_path: Option<String>,
    pub deleted: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct LoopConfigRow {
    pub workspace_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub memory_preload: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_profile: AgentProfile,
    pub model: Option<String>,
    pub close_after_settled_days: Option<i32>,
    pub settle_after_hours: Option<i32>,
    /// See [`moatless_types::loops::LoopContext::node_selector`].
    pub node_selector: Option<std::collections::BTreeMap<String, String>>,
    /// See [`moatless_types::loops::LoopContext::stop_when_done`].
    pub stop_when_done: Option<bool>,
    /// See [`moatless_types::loops::SubscriptionMatch::message_buffer_seconds`].
    pub message_buffer_seconds: Option<i32>,
    /// See [`NewLoopConfig::fallback_to_other_agent`].
    pub fallback_to_other_agent: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SubscriptionRow {
    pub adapter_kind: AdapterKind,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub event_filter: serde_json::Value,
    pub adapter_config: Option<serde_json::Value>,
    pub allow_direct_send: bool,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ScheduleRow {
    pub cron_expression: String,
    pub timezone: String,
    pub message_template: String,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub next_fire_at: DateTime<Utc>,
    pub locked_by: Option<String>,
    pub locked_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub(crate) enum LoopSourceRow {
    AdapterEvent(SubscriptionRow),
    Schedule(ScheduleRow),
    Manual,
}

#[derive(Debug, Clone)]
pub(crate) struct LoopRecord {
    pub loop_: LoopRow,
    pub config: LoopConfigRow,
    pub source: LoopSourceRow,
}

#[derive(Debug, Default)]
pub struct LoopListFilter<'a> {
    pub kind: Option<&'a str>,
    pub adapter_kind: Option<&'a str>,
}

// =============================================================================
// Write inputs — resolved values ready to persist. Callers own validation,
// scope resolution, and coalescing; the store owns only the SQL.
// =============================================================================

/// A fully-resolved new Loop, ready to insert.
pub struct NewLoop {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub active: bool,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub config: NewLoopConfig,
    pub source: NewLoopSource,
}

/// Resolved `loop_configs` values shared by create and update.
pub struct NewLoopConfig {
    pub workspace_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub memory_preload: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_profile: AgentProfile,
    pub model: Option<String>,
    pub close_after_settled_days: Option<i32>,
    pub settle_after_hours: Option<i32>,
    /// See [`moatless_types::loops::LoopContext::node_selector`].
    pub node_selector: Option<std::collections::BTreeMap<String, String>>,
    /// See [`moatless_types::loops::LoopContext::stop_when_done`].
    pub stop_when_done: Option<bool>,
    /// See [`moatless_types::loops::SubscriptionMatch::message_buffer_seconds`].
    pub message_buffer_seconds: Option<i32>,
    /// See [`moatless_types::loops::LoopContext::room`]. Written on insert
    /// only: `LOOP.md` is its one author, and an HTTP update leaves it alone.
    pub room: Option<RoomSpec>,
    /// Re-run a spawned Task's turn on the other harness once when the
    /// provider will not serve it: a usage limit, or a rate limit or an
    /// unavailable provider that outlasted retries. Read when the turn errors,
    /// not at spawn, so a toggle reaches Tasks already running.
    pub fallback_to_other_agent: bool,
}

pub enum NewLoopSource {
    AdapterEvent(NewAdapterEventSource),
    Schedule(NewScheduleSource),
    Manual,
}

pub struct NewAdapterEventSource {
    pub adapter_kind: String,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub event_filter: serde_json::Value,
    pub adapter_config: Option<serde_json::Value>,
    pub allow_direct_send: bool,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
}

pub struct NewScheduleSource {
    pub cron_expression: String,
    pub timezone: String,
    pub message_template: String,
    pub next_fire_at: DateTime<Utc>,
}

/// Resolved `loops` columns to update (identity + scope, not config/source).
pub struct LoopUpdate {
    pub name: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
}

/// A resolved source update. Unlike create, `adapter_kind` is immutable so the
/// adapter variant carries no kind.
pub enum UpdateLoopSource {
    AdapterEvent(AdapterSourceUpdate),
    Schedule(NewScheduleSource),
}

pub struct AdapterSourceUpdate {
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub event_filter: serde_json::Value,
    pub adapter_config: Option<serde_json::Value>,
    pub allow_direct_send: bool,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
}

/// A Git-owned Loop reconciled from a repository's `.moatless/loops/` tree,
/// keyed by `(source_repository_id, source_key)`.
pub struct ManagedLoopUpsert {
    pub source_repository_id: String,
    /// The Workspace the `workspace:` frontmatter names, resolved to an id. See
    /// [`moatless_types::loops::LoopContext::workspace_id`].
    pub workspace_id: String,
    pub source_key: String,
    pub source_config_path: String,
    pub trigger_name: String,
    pub kind: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub routing_mode: RoutingMode,
    pub prompt: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub memory_preload: Vec<String>,
    pub agent_profile: AgentProfile,
    pub model: Option<String>,
    pub close_after_settled_days: Option<i32>,
    pub settle_after_hours: Option<i32>,
    /// See [`moatless_types::loops::LoopContext::stop_when_done`]. Unlike
    /// `node_selector`, this one is declarable in `LOOP.md`: it only shortens a
    /// sandbox's life, so commit access buys no capacity.
    pub stop_when_done: Option<bool>,
    /// See [`moatless_types::loops::SubscriptionMatch::message_buffer_seconds`].
    pub message_buffer_seconds: Option<i32>,
    /// See [`moatless_types::loops::LoopContext::room`].
    pub room: Option<RoomSpec>,
    /// See [`NewLoopConfig::fallback_to_other_agent`].
    pub fallback_to_other_agent: bool,
    pub adapter: Option<NewAdapterEventSource>,
    pub schedule: Option<NewScheduleSource>,
}

#[derive(Debug, Default)]
pub struct ManagedLoopReconcileSummary {
    pub created: u32,
    pub updated: u32,
    pub removed: u32,
    pub overrides_skipped: u32,
    pub skipped_overrides: Vec<ManagedLoopOverrideSkip>,
}

#[derive(Debug)]
pub struct ManagedLoopOverrideSkip {
    pub source_key: String,
    pub deleted: bool,
}
