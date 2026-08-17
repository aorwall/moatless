//! Task domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};

use moatless_backend_core::scope::Scope;
use moatless_core::shared_models::AgentType;

pub use moatless_backend_core::models::{SandboxDesiredState, SandboxStopReason, TaskStatus};

/// How a task's checkout relates to other members of the same Sandbox.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

impl sqlx::Type<sqlx::Postgres> for CheckoutMode {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for CheckoutMode {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let value = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Self::try_from(value.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for CheckoutMode {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

/// Task visibility level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Private,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for Visibility {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            other => Err(format!("unknown Visibility: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for Visibility {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for Visibility {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let value = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Self::try_from(value.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for Visibility {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

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
pub struct Task {
    pub task_id: String,
    pub short_id: Option<String>,
    pub created_at: String,
    pub last_message_at: String,
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

impl Task {
    /// The task as a [`ScopedResource`] for access-control checks
    /// (`ScopeContext::can_read` / `can_control`). Ownership falls back to
    /// `created_by` for rows predating the `owner_user_id` column, matching the
    /// `COALESCE` in the DAO's scope filter.
    pub fn scoped_resource(&self) -> moatless_backend_core::scope::ScopedResource {
        moatless_backend_core::scope::ScopedResource::borrowed(
            self.scope,
            self.owner_user_id.as_deref().or(self.created_by.as_deref()),
        )
    }
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

/// Typed policy for task archive retention.
/// Runtime resources (Deployment, Service, IngressRoutes, **and the PVC**) are
/// always removed regardless of this policy; it governs S3 archives and
/// managed provider-local disk archives (opt-in Kubernetes node hostPaths and
/// Docker task directories).
///
/// The close, reap, and delete sites that choose a policy are tasks-band, so the
/// enum lives here; the lifecycle store, providers, and reconciler that persist
/// and apply it reach it on the legal downward `sandbox-control -> tasks` edge
/// (re-exported from `sandbox::lifecycle` so their existing paths are unchanged).
///
/// Persisted alongside `Removed` intent in the same atomic write so a crashed
/// remove can be retried with the *original* policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveRetention {
    /// Soft removal: keep archives so the task can be revived (reopen).
    RetainArchives,
    /// Permanent deletion: remove all archives too. Never superseded by a
    /// later message-wake — deletion wins.
    DeleteArchives,
}

impl ArchiveRetention {
    pub fn retains(self) -> bool {
        matches!(self, Self::RetainArchives)
    }

    /// The close-time policy from the `SANDBOX_RETAIN_STORAGE_ON_CLOSE` flag:
    /// retain archives (revivable) when set, delete them otherwise. One helper
    /// so the close/auto-close/adapter-close sites can't drift.
    pub fn on_close(retain_archives: bool) -> Self {
        if retain_archives {
            Self::RetainArchives
        } else {
            Self::DeleteArchives
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::RetainArchives => "retain_archives",
            Self::DeleteArchives => "delete_archives",
        }
    }

    /// Parse the persisted slug back into a policy. `None` for an unknown value.
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "retain_archives" => Some(Self::RetainArchives),
            "delete_archives" => Some(Self::DeleteArchives),
            _ => None,
        }
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
