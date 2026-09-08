//! Loop-owned execution configuration and schedule state.
use chrono::{DateTime, Utc};
use moatless_types::{agent::AgentType, access_control::Scope, loops::RoutingMode};

/// The `loop_configs` row a Loop spawns Tasks from.
#[derive(Debug, Clone)]
pub struct LoopSpawnConfig {
    pub id: String,
    pub repository_id: String,
    /// See [`moatless_types::loops::LoopContext::workspace_id`].
    pub workspace_id: Option<String>,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: AgentType,
    /// Per-Loop model override. See [`moatless_types::loops::LoopContext::model`].
    pub model: Option<String>,
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
