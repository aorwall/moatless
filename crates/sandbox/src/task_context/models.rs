use crate::agent::agent_state::AgentState;
use crate::agent::handle::AgentHandle;
use crate::agent::scheduled_job::ScheduledJobRegistry;
use crate::commands::CommandRegistry;
use crate::credentials::recovery::CredentialFailureDetector;
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
            agent,
            agent_handle,
            scheduled_jobs,
            commands,
            dispatch_lock: Default::default(),
            credential_failures,
            system_prompt_cache,
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
    #[error("failed to materialize isolated checkout: {0}")]
    CheckoutMaterialization(String),
}
