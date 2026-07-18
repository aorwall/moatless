//! Task domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};

use crate::shared_models::AgentType;
use moatless_backend_core::scope::Scope;

// Re-export enums from the shared crate.
pub use moatless_api_schemas::enums::{
    SandboxDesiredState, SandboxStopReason, TaskStatus, Visibility,
};

/// Default idle TTL in minutes for newly inserted tasks when the caller has no
/// access to platform config (e.g. integration runtimes). Matches the
/// `SANDBOX_IDLE_TIMEOUT_MINUTES` env-var default and the `tasks.ttl_minutes`
/// column default.
pub const DEFAULT_TTL_MINUTES: i32 = 15;

/// Default inactive TTL in days for newly inserted tasks when the caller has no
/// access to platform config (e.g. adapter runtimes). Matches the
/// `TASK_INACTIVE_TIMEOUT_DAYS` env-var default and the
/// `tasks.inactive_ttl_days` column default.
pub const DEFAULT_INACTIVE_TTL_DAYS: i32 = 7;

// =============================================================================
// Task (DB)
// =============================================================================

/// Full task row from the `tasks` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Task {
    pub task_id: String,
    pub short_id: Option<String>,
    pub created_at: String,
    pub last_message_at: String,
    pub agent_type: AgentType,
    pub repository_id: String,
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
    pub created_by: Option<String>,
    pub visibility: Visibility,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub created_at_db: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub append_system_prompt: Option<String>,
    pub sandbox_started_at: Option<DateTime<Utc>>,
    pub sandbox_desired_state: SandboxDesiredState,
    pub sandbox_stop_reason: Option<SandboxStopReason>,
    pub sandbox_desired_state_updated_at: DateTime<Utc>,
    pub headless: bool,
    /// Idle TTL in minutes. The reaper stops the sandbox when no activity has
    /// occurred for this many minutes. `0` means "indefinite — never reap".
    pub ttl_minutes: i32,
    /// Inactive TTL in days. The reaper auto-closes the task when no activity
    /// has occurred for this many days. `0` means "indefinite — never close".
    pub inactive_ttl_days: i32,
    /// Optional reference to the task template used to create this task.
    pub task_template_id: Option<String>,
    /// Trigger that spawned this task (route + schedule). Independent from any
    /// communication binding — schedule triggers never create a binding.
    pub trigger_id: Option<String>,
    /// Server names auto-disabled for this task after repeatedly crash-looping.
    /// JSON array of strings (e.g. `["storybook"]`). `None`/null means none
    /// disabled. Stored per-task so it survives stop/start and never leaks to
    /// other tasks sharing the same repository.
    pub disabled_servers: Option<serde_json::Value>,
}

impl Task {
    /// The task as a [`ScopedResource`] for access-control checks
    /// (`ScopeContext::can_read` / `can_control_task`). Ownership falls back to
    /// `created_by` for rows predating the `owner_user_id` column, matching the
    /// `COALESCE` in the DAO's scope filter.
    pub fn scoped_resource(&self) -> moatless_backend_core::scope::ScopedResource {
        moatless_backend_core::scope::ScopedResource::borrowed(
            self.scope,
            self.team_id.as_deref(),
            self.owner_user_id
                .as_deref()
                .or(self.created_by.as_deref()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_status_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::Open).unwrap(),
            "\"open\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Closed).unwrap(),
            "\"closed\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Error).unwrap(),
            "\"error\""
        );

        let parsed: TaskStatus = serde_json::from_str("\"open\"").unwrap();
        assert_eq!(parsed, TaskStatus::Open);
    }

    #[test]
    fn visibility_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&Visibility::Public).unwrap(),
            "\"public\""
        );
        assert_eq!(
            serde_json::to_string(&Visibility::Private).unwrap(),
            "\"private\""
        );

        let parsed: Visibility = serde_json::from_str("\"private\"").unwrap();
        assert_eq!(parsed, Visibility::Private);
    }

    #[test]
    fn task_status_try_from() {
        assert_eq!(TaskStatus::try_from("open"), Ok(TaskStatus::Open));
        assert!(TaskStatus::try_from("invalid").is_err());
    }

    #[test]
    fn visibility_try_from() {
        assert_eq!(Visibility::try_from("public"), Ok(Visibility::Public));
        assert!(Visibility::try_from("invalid").is_err());
    }

    #[test]
    fn sandbox_desired_state_try_from() {
        assert_eq!(
            SandboxDesiredState::try_from("running"),
            Ok(SandboxDesiredState::Running)
        );
        assert!(SandboxDesiredState::try_from("invalid").is_err());
    }

    #[test]
    fn sandbox_stop_reason_try_from() {
        assert_eq!(
            SandboxStopReason::try_from("idle_reaper"),
            Ok(SandboxStopReason::IdleReaper)
        );
        assert!(SandboxStopReason::try_from("invalid").is_err());
    }
}
