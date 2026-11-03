use async_trait::async_trait;
use moatless_types::agent::AgentProfile;
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
    pub agent_profile: AgentProfile,
    /// The model the spawned Task runs on. `None` keeps the platform default.
    pub model: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    /// The identity the Task is attributed to.
    pub created_by: Option<String>,
    /// See [`moatless_types::task::Task::reporter_user_id`].
    pub reporter_user_id: Option<String>,
    /// The account the Task acts as, even when it is `created_by`: a set value
    /// pins it over whoever later drives the Sandbox. A Workspace that names
    /// its own run-as identity still outranks it.
    pub run_as_user_id: Option<String>,
    pub visibility: Visibility,
    /// The Loop that fired.
    pub loop_id: String,
    pub append_system_prompt: Option<String>,
    pub ttl_minutes: i32,
    pub close_after_settled_days: i32,
    pub settle_after_hours: Option<i32>,
    /// See [`moatless_types::loops::LoopContext::node_selector`]. `None` leaves
    /// the sandbox on whatever its Workspace declares.
    pub node_selector: Option<std::collections::BTreeMap<String, String>>,
    /// See [`moatless_types::loops::LoopContext::stop_when_done`].
    pub stop_when_done: bool,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    /// See [`moatless_types::task::Task::memory_preload`].
    pub memory_preload: Vec<String>,
    /// A peer Task to link this one to once it exists.
    pub related_to: Option<RelatedTask>,
    /// Merged into the Task's own creation event. The task band forwards it
    /// untouched.
    pub event_metadata: serde_json::Value,
    /// The Task Room the Loop declares. The task band ignores it: the opener
    /// that decorates it for every Loop path requests the room once the Task
    /// exists.
    pub room: Option<moatless_types::loops::RoomSpec>,
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
