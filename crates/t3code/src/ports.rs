//! What this module needs from the rest of the backend in order to write.
//!
//! Everything else here is a database read, which is why the module's other
//! dependencies are DAOs. Sending a message is not: it persists a row, follows
//! a Task on someone's behalf, and wakes a Sandbox. `SandboxConnector` is the
//! same kind of narrow port for the one thing behind this boundary that is not a
//! query, so the
//! compatibility layer keeps translating rather than learning how Moatless
//! works.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_api_schemas::agent::AgentDescriptor;
use moatless_api_schemas::repository::BranchListResponse;
use moatless_backend_core::scope::ScopeContext;

use moatless_api_schemas::shared::AgentMode;

/// Why a command was not carried out.
///
/// Every variant is an *anticipated* outcome and reaches the client as a typed
/// failure it can render. A defect is reserved for what it means — a bug in this
/// server — so a person is never told the server is broken because they clicked
/// a control it does not implement.
#[derive(Debug, Clone)]
pub enum CommandRefusal {
    /// The command exists in T3's contract and this surface does not implement
    /// it.
    Unsupported { command: String, reason: String },
    /// The Task is gone, closed, or not one this viewer may write to. The three
    /// are deliberately not distinguished: telling somebody a Task they cannot
    /// read exists is itself a disclosure.
    NotWritable { task_id: String },
    /// The request was understood and is wrong — a model we never published, a
    /// provider the Thread cannot run, a Workspace with no repository.
    Rejected { message: String },
    /// The write was attempted and failed. The cause is logged, not sent.
    Failed { message: String },
}

impl CommandRefusal {
    pub fn message(&self) -> String {
        match self {
            Self::Unsupported { command, reason } => {
                format!("`{command}` is not supported by this server. {reason}")
            }
            Self::NotWritable { task_id } => {
                format!("Thread {task_id} cannot be written to from here.")
            }
            Self::Rejected { message } | Self::Failed { message } => message.clone(),
        }
    }
}

/// One turn, as this surface understood it.
pub struct ThreadTurn {
    pub task_id: String,
    pub text: String,
    /// The id the client minted. Persisted verbatim so the client can retire
    /// its optimistic copy.
    pub message_uuid: String,
    /// A catalog model id, already translated out of T3's published slug.
    pub model: Option<String>,
    pub agent_mode: Option<AgentMode>,
    /// The reasoning effort selected for this turn, in the agent's own
    /// vocabulary. `None` leaves the agent on its own default.
    pub effort: Option<String>,
}

/// A new Thread, as this surface understood it.
pub struct NewThread {
    /// The id the client minted and has already navigated to.
    pub task_id: String,
    /// The T3 Project, which is a Moatless Workspace.
    pub workspace_id: String,
    /// The provider instance the person picked, which decides which agent runs
    /// the thread. The transport within that kind is not selectable from T3.
    pub instance: String,
    pub title: Option<String>,
    /// A catalog model id.
    pub model: Option<String>,
    /// The branch the person picked, if any. `None` falls back to the primary
    /// repository's default branch when the Task is created.
    pub branch: Option<String>,
    /// The first message, when the command carried one. Absent means the Task
    /// is created without a Sandbox.
    pub first_message: Option<ThreadTurn>,
}

/// A title and model change, as this surface understood it.
pub struct ThreadMeta {
    pub task_id: String,
    pub title: Option<String>,
    pub model: Option<String>,
}

/// An answer to a question the agent asked.
pub struct QuestionAnswer {
    pub task_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub payload: Option<serde_json::Value>,
}

/// The agent catalog, as this viewer can use it.
#[async_trait]
pub trait AgentCatalogSource: Send + Sync {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor>;
}

/// Why a repository's branches could not be listed.
///
/// Split by what the branch picker is told: [`Self::None`] is answered with an
/// empty list, the other two are surfaced or logged where the frame is built.
#[derive(Debug)]
pub enum BranchListError {
    /// Nothing to list: the repository is gone, or it is not one this viewer
    /// can be shown. Indistinguishable on purpose, so a guessed id discloses
    /// nothing.
    None,
    /// The repository is there and its branches could not be fetched — no token
    /// connected, or the git host refused. Surfaced in the picker verbatim,
    /// which is where the "connect your account" prompt has to land.
    Unavailable(String),
    /// This deployment failed to answer. The cause is logged, not sent.
    Failed(sqlx::Error),
}

/// A repository's branches, read from its git host.
///
/// The one read here that is not a database query: a draft thread's branch
/// picker lists the primary repository's branches before any Sandbox exists,
/// the same list the Moatless UI shows, which means calling out to the git host
/// through the same path Moatless's own `GET /branches` uses.
#[async_trait]
pub trait RepositoryBranchSource: Send + Sync {
    async fn list_branches(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
    ) -> Result<BranchListResponse, BranchListError>;
}

/// The pull request a Task is bound to, in the shape the status surface reports.
///
/// Base and head are non-optional because T3's `pr` field requires both: a
/// binding whose persisted blob predates them yields no `BoundPullRequest` at
/// all, so the toolbar shows no pull request rather than one with blank
/// branches.
pub struct BoundPullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub base_ref: String,
    pub head_ref: String,
    /// The binding's normalized state kind: `open`, `draft`, `merged` or
    /// `closed`. The status surface collapses `draft` to `open` for T3's
    /// three-value set.
    pub state_kind: String,
}

/// A pull request resolved from a reference, in the shape T3's
/// `GitResolvedPullRequest` reports: base and head are the branch names, and
/// `state` is one of `open`, `closed` or `merged`.
pub struct ResolvedPullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub base_branch: String,
    pub head_branch: String,
    pub state: String,
}

/// Why a pull-request reference could not be resolved.
///
/// Split by what the attach-PR dialog is told: [`Self::NotFound`] is a reference
/// that names no pull request the viewer can reach, [`Self::Unavailable`] is a
/// reachable repository whose host refused or has no GitHub credential — both
/// surfaced verbatim — and [`Self::Failed`] is this deployment's own failure,
/// logged rather than sent.
#[derive(Debug)]
pub enum PullRequestError {
    NotFound,
    Unavailable(String),
    Failed(String),
}

/// The pull request a Task is bound to, and resolution of references against a
/// repository's git host.
///
/// A database read like the DAOs for the bound case, but one that has to reach
/// into an adapter's persisted status blob — and the reference-resolution and
/// head-lookup calls reach the git host — so it is a port rather than a bare
/// DAO: `crates/t3code` stays free of any GitHub knowledge, and how a pull
/// request is recovered or resolved lives on the backend side of this boundary.
#[async_trait]
pub trait BoundPullRequestSource: Send + Sync {
    /// The pull request `task_id` is bound to, or `None` when it has none the
    /// surface can report yet. Access to the Task is checked before this is
    /// called, so it takes no viewer.
    async fn bound_pull_request(&self, task_id: &str) -> Option<BoundPullRequest>;

    /// Resolve a pull-request reference against a repository, for the attach-PR
    /// dialog. `reference` is already trimmed and is a bare number or
    /// `owner/repo#N`.
    async fn resolve_pull_request(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
        reference: &str,
    ) -> Result<ResolvedPullRequest, PullRequestError>;

    /// The open pull request whose head branch is `head`, if any — the signal a
    /// Task created on a PR's head branch is bound to it. `None` on any failure,
    /// so a creation that cannot reach the host is created unbound rather than
    /// refused.
    async fn open_pull_request_for_head(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
        head: &str,
    ) -> Option<ResolvedPullRequest>;
}

/// The viewer's connected GitHub identity, for the Source Control panel's auth
/// badge. `account` is the GitHub login the credential recorded, when known.
pub struct GithubIdentity {
    pub account: Option<String>,
}

/// Whether the viewer has a usable GitHub credential, read from their own
/// provider-token row rather than from how this session authenticated.
///
/// A port because `crates/t3code` stays free of credential storage: which rows
/// count as a usable GitHub connection, and how a stale one is told apart, live
/// on the backend side of this boundary.
#[async_trait]
pub trait SourceControlAuthSource: Send + Sync {
    /// The viewer's connected GitHub identity, or `None` when they have no
    /// usable GitHub credential.
    async fn github_identity(&self, viewer: &ScopeContext) -> Option<GithubIdentity>;
}

/// Everything a T3 command can ask Moatless to do.
#[async_trait]
pub trait ThreadCommandSink: Send + Sync {
    async fn send_turn(
        &self,
        viewer: &ScopeContext,
        turn: ThreadTurn,
    ) -> Result<(), CommandRefusal>;

    async fn create_thread(
        &self,
        viewer: &ScopeContext,
        thread: NewThread,
    ) -> Result<(), CommandRefusal>;

    async fn update_meta(
        &self,
        viewer: &ScopeContext,
        meta: ThreadMeta,
    ) -> Result<(), CommandRefusal>;

    /// Stop whatever the agent is doing. T3 distinguishes interrupting a turn
    /// from stopping a session; Moatless does not, and both arrive here.
    async fn stop(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Archive a thread, which in Moatless is closing the Task: the row leaves
    /// the sidebar, the Sandbox is removed, and inbound comments stop waking
    /// it. Children are left open — the cascade is a choice the Moatless UI
    /// asks about, and there is nothing here to ask with.
    async fn archive(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Unarchive a thread, which is reopening the Task. No Sandbox is started;
    /// the next message does that.
    async fn unarchive(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    async fn settle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    async fn unsettle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    async fn snooze(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        snoozed_until: DateTime<Utc>,
    ) -> Result<(), CommandRefusal>;

    async fn unsnooze(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    async fn answer_question(
        &self,
        viewer: &ScopeContext,
        answer: QuestionAnswer,
    ) -> Result<(), CommandRefusal>;

    /// Record onto the Task the branch a git action created, closing the drift
    /// between the checkout — moved to `t3/<slug>` — and the Task row, which
    /// still names the branch the Task was created on.
    async fn set_branch(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        branch: &str,
    ) -> Result<(), CommandRefusal>;
}
