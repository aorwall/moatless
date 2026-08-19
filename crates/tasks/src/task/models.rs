//! Persisted shapes for the task band.
//!
//! The domain types live in `moatless_types::task`. What is here is how
//! they are stored: `tasks.created_at` is a text column, `created_at_db` is
//! a second timestamp the domain does not model, and the enum columns are
//! text.

use chrono::{DateTime, Utc};

use moatless_backend_core::db_enum::decode_optional;
use moatless_types::access_control::Scope;
use moatless_types::agent::AgentType;

// The task band's vocabulary surface: call sites keep naming these through
// `task::models`, which is where a reader of this crate looks for them.
pub use moatless_types::sandbox::{SandboxDesiredState, SandboxStopReason};
pub use moatless_types::task::{CheckoutMode, Task, TaskStatus, Visibility};

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

/// The timestamp for a Sandbox that has never received durable lifecycle
/// intent. Every real Message timestamp sorts after this sentinel.
pub fn sandbox_intent_epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from(std::time::UNIX_EPOCH)
}

// =============================================================================
// Task (DB)
// =============================================================================

/// Full task row from the `tasks` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskRow {
    pub task_id: String,
    pub short_id: Option<String>,
    pub created_at: String,
    pub last_message_at: String,
    #[sqlx(try_from = "String")]
    pub agent_type: AgentType,
    pub repository_id: String,
    /// Workspace this task runs against (Phase A: nullable during the additive
    /// transition; backfilled to the repository's shadow workspace). When set,
    /// provisioning resolves the repository composition via the workspace;
    /// otherwise it falls back to `repository_id`.
    pub workspace_id: Option<String>,
    /// Sandbox this task belongs to. A task with no row here has never been
    /// started; the ordinary first start mints a sandbox whose id equals the
    /// task id.
    pub sandbox_id: Option<String>,
    /// Checkout relationship to other members. `None` is read as shared for
    /// rows predating shared-sandbox membership.
    pub checkout_mode_raw: Option<String>,
    #[sqlx(try_from = "String")]
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
    #[sqlx(try_from = "String")]
    pub visibility: Visibility,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at_db: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub append_system_prompt: Option<String>,
    pub headless: bool,
    /// Idle TTL in minutes. The reaper stops the sandbox when no activity has
    /// occurred for this many minutes. `0` means "indefinite — never reap".
    pub ttl_minutes: i32,
    /// Inactive TTL in days. The reaper auto-closes the task when no activity
    /// has occurred for this many days. `0` means "indefinite — never close".
    pub inactive_ttl_days: i32,
    /// Loop that spawned this task (subscription + schedule). Independent from any
    /// communication binding — schedule loops never create a binding.
    pub loop_id: Option<String>,
    /// Skills every fresh agent session on this task activates. Set by the Loop
    /// that spawned it; re-frontloaded after a restart or a start-fresh send,
    /// because the transcript that carried them is gone.
    pub skills: Vec<String>,
}

/// Parsing a row into the domain Task. A column no variant names, or a
/// timestamp that is not RFC3339, is a decode error naming the column — the
/// text columns hold values this application wrote.
impl TryFrom<TaskRow> for Task {
    type Error = sqlx::Error;

    fn try_from(row: TaskRow) -> Result<Self, Self::Error> {
        Ok(Self {
            created_at: parse_timestamp("created_at", &row.created_at)?,
            last_message_at: parse_timestamp("last_message_at", &row.last_message_at)?,
            checkout_mode: decode_optional("checkout_mode", row.checkout_mode_raw)?,
            task_id: row.task_id,
            short_id: row.short_id,
            agent_type: row.agent_type,
            repository_id: row.repository_id,
            workspace_id: row.workspace_id,
            sandbox_id: row.sandbox_id,
            status: row.status,
            total_cost_usd: row.total_cost_usd,
            site_url: row.site_url,
            template_id: row.template_id,
            vibe: row.vibe,
            name: row.name,
            description: row.description,
            site_id: row.site_id,
            site_provider: row.site_provider,
            branch: row.branch,
            model: row.model,
            created_by: row.created_by,
            visibility: row.visibility,
            scope: row.scope,
            owner_user_id: row.owner_user_id,
            updated_at: row.updated_at,
            append_system_prompt: row.append_system_prompt,
            headless: row.headless,
            ttl_minutes: row.ttl_minutes,
            inactive_ttl_days: row.inactive_ttl_days,
            loop_id: row.loop_id,
            skills: row.skills,
        })
    }
}

/// `tasks.created_at` and `last_message_at` are text columns holding RFC3339.
fn parse_timestamp(column: &'static str, value: &str) -> Result<DateTime<Utc>, sqlx::Error> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|error| sqlx::Error::ColumnDecode {
            index: column.to_string(),
            source: Box::new(error),
        })
}

/// The durable Sandbox lifecycle state for one Task, read from the `sandboxes`
/// table to populate task responses. Absence of a Sandbox row is the lazy
/// converged-Stopped default.
#[derive(Debug, Clone)]
pub struct SandboxIntentView {
    pub started_at: Option<DateTime<Utc>>,
    pub desired_state: SandboxDesiredState,
    pub stop_reason: Option<SandboxStopReason>,
    pub desired_state_updated_at: DateTime<Utc>,
}

impl Default for SandboxIntentView {
    /// A Task with no Sandbox row: never requested = converged Stopped, no start
    /// timestamp, epoch intent timestamp.
    fn default() -> Self {
        Self {
            started_at: None,
            desired_state: SandboxDesiredState::Stopped,
            stop_reason: None,
            desired_state_updated_at: sandbox_intent_epoch(),
        }
    }
}

impl SandboxIntentView {
    /// Generation allocation may create the row before lifecycle persists
    /// intent. The epoch sentinel keeps that crash window distinct from an
    /// explicit Stopped or Removed request.
    pub fn has_explicit_intent(&self) -> bool {
        self.desired_state_updated_at != sandbox_intent_epoch()
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
