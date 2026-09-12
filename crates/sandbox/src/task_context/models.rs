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
/// A warm sandbox starts with one context keyed by the boot task. Joined tasks
/// get their own context so agent status, turns, prompts, scheduled jobs, and
/// dispatch single-flight state stay isolated by task.
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
    /// Agent type this member's harness was built from. A shared sandbox runs
    /// one harness per member, so this is the member's own selection and not the
    /// pod's `AGENT_TYPE`, which names only the member it booted with. `None`
    /// when neither was set.
    pub agent_type: Option<String>,
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
    /// Uuids of the rows this pod queued for itself, through
    /// `dispatcher::dispatch_self_queued`. A parked `AskUserQuestion` or
    /// `ExitPlanMode` waits on a person, so `dispatcher::resolve_pending_prompt`
    /// never acknowledges a row from here as their answer.
    ///
    /// In memory deliberately: a parked prompt dies with its pod, so a row the
    /// replacement finds still pending is ordinary input to it.
    ///
    /// Private: [`Self::mark_self_queued`], [`Self::split_self_queued`] and
    /// [`Self::forget_self_queued`] are the only way in.
    self_queued: Arc<Mutex<HashSet<String>>>,
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
        agent_type: Option<String>,
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
            agent_type,
            agent,
            agent_handle,
            scheduled_jobs,
            commands,
            dispatch_lock: Default::default(),
            self_queued: Default::default(),
            credential_failures,
            system_prompt_cache,
        }
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

    /// Forget rows the harness has accepted, so a pod running a `/loop` does not
    /// accumulate one entry per tick for its whole life.
    ///
    /// Call this only once acceptance is confirmed. A row that was dispatched but
    /// not taken is still pending and still a wake, and forgetting it early lets
    /// the redispatch answer a prompt parked in between with it.
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
    /// accepted set — the reconciler retries the join every 30s and its log line
    /// is the only place this surfaces.
    #[error("{0}")]
    UnsupportedAgentType(String),
    #[error("failed to materialize isolated checkout: {0}")]
    CheckoutMaterialization(String),
}
