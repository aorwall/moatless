use crate::agent::agent_state::AgentState;
use crate::agent::handle::AgentHandle;
use crate::agent::scheduled_job::ScheduledJobRegistry;
use crate::commands::CommandRegistry;
use crate::credentials::recovery::CredentialFailureDetector;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Per-task runtime state inside a sandbox process.
///
/// A warm sandbox starts with one context keyed by the boot task; each joined
/// task gets its own, isolating agent status, turns, prompts, jobs and dispatch
/// state.
#[derive(Clone)]
pub struct TaskContext {
    /// Task this context serves.
    pub task_id: String,
    /// Workspace root used by this task.
    pub workspace_dir: String,
    /// Primary checkout used by this task.
    pub checkout_dir: String,
    /// Checkout relationship requested by the backend for this member.
    pub checkout_mode: CheckoutMode,
    /// Agent profile this member's harness was built from — the member's own
    /// selection, not the pod's `AGENT_PROFILE` (which only names the boot
    /// member). `None` when neither was set.
    pub agent_profile: Option<String>,
    /// Agent execution state (status, session, cancel token, pending prompts).
    pub agent: Arc<AgentState>,
    /// Agent command handle — send messages via `agent_handle.send()`.
    pub agent_handle: AgentHandle,
    /// Registry of deferred-work jobs (Monitor crons + `ScheduleWakeup` timers)
    /// spawned by the permission handler / agent lifecycle service.
    pub scheduled_jobs: Arc<ScheduledJobRegistry>,
    /// Commands this task started through `moat cmd`. The only source of
    /// `busy_processes`, so a process started any other way cannot defer an
    /// idle stop.
    pub commands: Arc<CommandRegistry>,
    /// Single-flight guard for this task's pending-message dispatch.
    pub dispatch_lock: Arc<Mutex<()>>,
    /// Uuids of rows this pod queued for itself.
    ///
    /// `resolve_pending_prompt` never treats one as a person's answer to a
    /// parked `AskUserQuestion`/`ExitPlanMode`; kept in memory only, since a
    /// parked prompt dies with its pod.
    self_queued: Arc<Mutex<HashSet<String>>>,
    /// A `<sandbox-restart-note>` the next dispatched turn leads with, set when
    /// a queued message took precedence over replaying an interrupted turn.
    restart_note: Arc<std::sync::Mutex<Option<String>>>,
    /// Tracks credential failures observed on this task's agent stderr.
    pub credential_failures: CredentialFailureDetector,
    /// Last system prompt fetched for this task. Used as a fallback when
    /// `BackendClient::get_system_prompt` fails on a subsequent run.
    pub system_prompt_cache: Arc<RwLock<Option<String>>>,
}

impl TaskContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_id: String,
        workspace_dir: String,
        checkout_dir: String,
        checkout_mode: CheckoutMode,
        agent_profile: Option<String>,
        agent: Arc<AgentState>,
        agent_handle: AgentHandle,
        scheduled_jobs: Arc<ScheduledJobRegistry>,
        commands: Arc<CommandRegistry>,
        credential_failures: CredentialFailureDetector,
        system_prompt_cache: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            task_id,
            workspace_dir,
            checkout_dir,
            checkout_mode,
            agent_profile,
            agent,
            agent_handle,
            scheduled_jobs,
            commands,
            dispatch_lock: Default::default(),
            self_queued: Default::default(),
            restart_note: Default::default(),
            credential_failures,
            system_prompt_cache,
        }
    }

    /// Hold `note` for the next dispatched turn. See [`Self::restart_note`].
    pub(crate) fn set_restart_note(&self, note: String) {
        *self
            .restart_note
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(note);
    }

    /// Take the held note, leaving none for a later turn.
    pub(crate) fn take_restart_note(&self) -> Option<String> {
        self.restart_note
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Record a row this pod queued for itself. See [`Self::self_queued`].
    pub async fn mark_self_queued(&self, uuid: String) {
        self.self_queued.lock().await.insert(uuid);
    }

    /// Split a delivery's uuids into `(self_queued, rest)`. The backend
    /// coalesces every pending row into one delivery, so a wake and a person's
    /// answer arrive together and have to be told apart row by row.
    pub async fn split_self_queued(&self, uuids: &[String]) -> (Vec<String>, Vec<String>) {
        let self_queued = self.self_queued.lock().await;
        uuids
            .iter()
            .cloned()
            .partition(|uuid| self_queued.contains(uuid))
    }

    /// Forget rows the harness has accepted, so a `/loop` pod does not
    /// accumulate one entry per tick.
    ///
    /// Call only once acceptance is confirmed — forgetting a still-pending row
    /// early lets a redispatch answer a prompt parked in between with it.
    pub async fn forget_self_queued(&self, uuids: &[String]) {
        let mut self_queued = self.self_queued.lock().await;
        for uuid in uuids {
            self_queued.remove(uuid);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutMode {
    Shared,
    Isolated,
}

impl CheckoutMode {
    pub(super) fn parse(raw: Option<&str>) -> Result<Self, RegisterTaskContextError> {
        match raw.unwrap_or("shared") {
            "shared" | "" => Ok(Self::Shared),
            "isolated" => Ok(Self::Isolated),
            other => Err(RegisterTaskContextError::InvalidCheckoutMode(
                other.to_string(),
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Isolated => "isolated",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TaskContextLookupError {
    #[error("task is not registered in this sandbox process: {0}")]
    NotFound(String),
    #[error("task id is required when multiple tasks are registered")]
    Ambiguous,
    #[error("sandbox has no registered task contexts")]
    Empty,
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterTaskContextError {
    #[error("task_id is required")]
    EmptyTaskId,
    #[error("unsupported checkout_mode: {0}")]
    InvalidCheckoutMode(String),
    /// Carries `resolve_agent_harness`'s own diagnostic, which names the
    /// accepted set — the reconciler retries the join every 30s and its log
    /// line is the only place this surfaces.
    #[error("{0}")]
    UnsupportedAgentProfile(String),
    #[error("failed to materialize isolated checkout: {0}")]
    CheckoutMaterialization(String),
}
