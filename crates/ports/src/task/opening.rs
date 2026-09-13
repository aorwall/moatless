use async_trait::async_trait;
use moatless_types::agent::AgentType;
use moatless_types::store::StoreError;
use moatless_types::task::RelatedTask;
use moatless_types::task::Visibility;

/// A Task a Loop asks the task band to open.
///
/// The provider path's shape: every Task opened this way is headless,
/// provisions a sandbox, and publishes an `Open` event, so none of those is a
/// field here.
#[derive(Debug, Clone)]
pub struct NewTask {
    /// The Workspace the Loop names. Its primary placement is the repository
    /// the Task runs against.
    pub workspace_id: String,
    pub agent_type: AgentType,
    /// The model the spawned Task runs on. `None` keeps the platform default.
    pub model: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    /// The identity the Task is attributed to.
    pub created_by: Option<String>,
    pub visibility: Visibility,
    /// The Loop that fired.
    pub loop_id: String,
    pub append_system_prompt: Option<String>,
    pub ttl_minutes: i32,
    pub inactive_ttl_days: i32,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    /// A peer Task to link this one to once it exists.
    pub related_to: Option<RelatedTask>,
    /// Merged into the Task's own creation event. The task band forwards it
    /// untouched.
    pub event_metadata: serde_json::Value,
}

/// What became of the sandbox a freshly-opened Task asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxProvision {
    Started,
    Failed,
}

/// A Task that now exists.
#[derive(Debug, Clone)]
pub struct TaskOpened {
    pub task_id: String,
    pub provision: SandboxProvision,
}

/// Why a Task could not be opened.
///
/// [`Self::Refused`] is the preflight answer — the identity would run as cannot
/// reach what it needs — the one variant a caller reports to a person rather
/// than logging.
#[derive(Debug, thiserror::Error)]
pub enum TaskOpenError {
    #[error("repository {0} not found")]
    RepoNotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Opening a Task from the provider path.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TaskOpener: Send + Sync {
    async fn open(&self, req: NewTask) -> Result<TaskOpened, TaskOpenError>;
}
