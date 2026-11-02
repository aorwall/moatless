//! What this module needs from the rest of the backend in order to write.
//!
//! Everything else here is a database read, so the module's other dependencies
//! are DAOs. `SandboxConnector` is the same kind of narrow port for the one
//! write that is not a query.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_api_schemas::agent::AgentDescriptor;
use moatless_types::access_control::ScopeContext;
use moatless_types::pull_request_conversation::{
    FileChangeType, FileContentsPair, PullRequestActivity, PullRequestStatus,
    ReviewThreadCommentPage,
};
use moatless_types::store::StoreError;
use moatless_types::task::{TaskRelationType, Visibility};
use moatless_workspace_types::schemas::repository::BranchListResponse;
use uuid::Uuid;

use moatless_types::agent::{AgentMode, Harness};

/// The Workspace Scripts a Task runs against, read through the Task.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WorkspaceScriptSource: Send + Sync {
    /// `None` when the viewer cannot read the Task, it does not exist, or its
    /// Workspace is gone: the same answer as everywhere else a scoped read
    /// finds nothing. `Some(vec)`, maybe empty, when it is readable.
    async fn scripts_for_task(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<Vec<moatless_workspace_types::workspace::ScriptConfig>>, StoreError>;
}

/// Why a command was not carried out — an *anticipated* outcome the client can
/// render as a typed failure. A defect means a bug in this server, not this.
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

/// One attachment the client sent with a turn, in either form the contract
/// allows.
///
/// Which form arrives is decided by the `attachmentUploads` capability
/// [`descriptor`](crate::rpc::config::descriptor) reports; both keep working as
/// a union, not a replacement.
pub enum TurnAttachment {
    /// Bytes sent inline in the command. [`commands`](super::commands) has
    /// already validated the payload; `mime_type` and the byte length are what
    /// the bytes actually are, not what the client declared.
    Inline {
        /// The name the person's file had, for the transcript and the download.
        filename: String,
        mime_type: String,
        bytes: Vec<u8>,
    },
    /// An upload that already exists, named by the id
    /// `attachments.createUploadUrl` minted for it. Carries no bytes.
    ///
    /// An id proves nothing alone; `kind` is checked against the stored row —
    /// see [`AttachmentKind::accepts`].
    Uploaded {
        attachment_id: String,
        kind: AttachmentKind,
    },
}

/// Which member of the attachment union a turn claims an upload under.
///
/// The two differ in what they accept, so the claim decides which limits the
/// stored file must satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    Image,
    File,
}

impl AttachmentKind {
    /// Whether a stored upload could have been minted under this member.
    ///
    /// An id alone is not provenance: a file stored via Moatless's own
    /// multipart route could otherwise be claimed past what
    /// `attachments.createUploadUrl` would have signed for.
    #[must_use]
    pub fn accepts(self, mime_type: &str, size_bytes: i64) -> bool {
        let mime_type = mime_type.trim().to_ascii_lowercase();
        match self {
            Self::Image => {
                crate::attachments::SUPPORTED_IMAGE_MIME_TYPES.contains(&mime_type.as_str())
                    && (1..=crate::attachments::MAX_IMAGE_BYTES).contains(&size_bytes)
            }
            Self::File => (1..=crate::attachments::MAX_FILE_BYTES).contains(&size_bytes),
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
    /// Another harness than the Task's, switched to before the turn is sent.
    pub harness: Option<Harness>,
    pub agent_mode: Option<AgentMode>,
    /// The reasoning effort selected for this turn, in the agent's own
    /// vocabulary. `None` leaves the agent on its own default.
    pub effort: Option<String>,
    /// The images that came with the turn, in the order the composer held them.
    /// Empty for the overwhelming majority of turns.
    pub attachments: Vec<TurnAttachment>,
    /// The skills to activate for this turn, in the order the person picked
    /// them. Read out of the `$name` mentions the composer leaves in
    /// [`Self::text`] — `skills::activated`.
    pub skills: Vec<String>,
    /// Hold the turn until the active one ends rather than steering into it:
    /// the command's `dispatchMode: "queue"`.
    pub queue: bool,
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
    /// A catalog model id.
    pub model: Option<String>,
    /// The branch the person picked, if any. `None` falls back to the primary
    /// repository's default branch when the Task is created.
    pub branch: Option<String>,
    /// The first message, when the command carried one.
    pub first_message: Option<ThreadTurn>,
    /// Whether to start the Sandbox for a Task created with no first message (a
    /// create carrying one always provisions) — for wanting the Sandbox for
    /// something other than an agent turn, such as running a Script from a
    /// draft.
    pub provision: bool,
}

/// A title and model change, as this surface understood it.
pub struct ThreadMeta {
    pub task_id: String,
    /// A rename. Absent for the client's auto-title, which this surface drops.
    pub title: Option<String>,
    /// A catalog model id of `harness`, or of the Task's own harness.
    pub model: Option<String>,
    /// Another harness than the Task's, to switch to.
    pub harness: Option<Harness>,
}

/// The pull request a link or an unlink command named, as this surface
/// understood it.
///
/// `source` and `url` are not carried: Moatless derives both from the path that
/// wrote the Binding and from host/repository/number.
pub struct ThreadPullRequestRef {
    pub task_id: String,
    /// The git host T3 keys the link by. Only `github.com` can be bound.
    pub host: String,
    /// `owner/name`.
    pub repository: String,
    pub number: i64,
}

/// A fork, as this surface understood it.
pub struct ForkThread {
    pub side_chat: bool,
    pub title: Option<String>,
    /// The id the client minted for the fork and has already navigated to — the
    /// same convention [`NewThread::task_id`] follows.
    pub new_task_id: String,
    pub source_task_id: String,
    /// The source Task's turn the copy stops after. `None` cuts after its
    /// newest turn that reached a terminal status.
    pub at_turn: Option<i32>,
    /// The fork's own first message. `None` creates it idle.
    pub message: Option<String>,
    /// Join the source Task's Sandbox on its shared checkout instead of
    /// provisioning one from a copy of its workspace archive.
    pub same_sandbox: bool,
    /// A catalog model id, already translated out of T3's published slug.
    pub model: Option<String>,
    /// Another harness than the source's, which the fork runs.
    pub harness: Option<Harness>,
    pub agent_mode: Option<AgentMode>,
    /// The reasoning effort for the first turn, in the agent's own vocabulary.
    /// `None` leaves the agent on its own default.
    pub effort: Option<String>,
}

/// An answer to a question the agent asked.
pub struct QuestionAnswer {
    pub task_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub payload: Option<serde_json::Value>,
}

/// One change to the turns a thread has queued, each named by its message id.
pub enum QueueChange {
    Cancel {
        message_uuid: String,
    },
    /// `attachments` replaces the message's files when `Some`.
    Edit {
        message_uuid: String,
        text: String,
        attachments: Option<Vec<TurnAttachment>>,
    },
    /// Move before `before`, or to the end of the queue.
    Reorder {
        message_uuid: String,
        before: Option<String>,
    },
    /// Deliver what a stop left waiting.
    Resume,
}

/// The agent catalog, as this viewer can use it.
#[async_trait]
pub trait AgentCatalogSource: Send + Sync {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor>;
}

/// One Task that names another as its parent.
///
/// Carries no access decision: whether the child may be shown is settled by
/// re-reading it through
/// [`ConversationDao::readable_by_ids`](crate::dao::ConversationDao::readable_by_ids).
pub struct ChildTask {
    pub task_id: String,
    /// Which parent edge this is. Both make the child a child; they differ in
    /// what a reader should call it.
    pub relation: TaskRelationType,
}

/// The Tasks that name a Task as their parent.
///
/// A port, not a query: relations are the Task band's vocabulary. Access is not
/// this port's business — it answers with every edge and the caller narrows to
/// what the viewer may read.
#[async_trait]
pub trait TaskRelationSource: Send + Sync {
    async fn children(&self, parent_task_id: &str) -> Result<Vec<ChildTask>, StoreError>;

    /// The edge naming the Task `child_task_id` was spawned by or forked from;
    /// `None` for a Task with neither.
    async fn parent(&self, child_task_id: &str) -> Result<Option<ParentTask>, StoreError>;
}

/// One Task's parent edge, as [`TaskRelationSource::parent`] answers it.
pub struct ParentTask {
    pub side_chat: bool,
    pub task_id: String,
    pub relation: TaskRelationType,
    /// The source turn a fork was cut after, from the edge's context. `None`
    /// for a fork of the whole thread, and for every other relation.
    pub at_turn: Option<i32>,
}

/// One skill a Workspace's agent would load, as the composer's `/` menu needs
/// it.
pub struct WorkspaceSkill {
    pub name: String,
    pub description: String,
    /// The plugin that ships it, for a skill that comes from one. `None` means
    /// the Workspace repository's own tree.
    pub plugin_name: Option<String>,
}

/// The skills a Workspace's agent would load, in the order they are offered.
///
/// A port because it reaches a git host: `.claude/skills` is fetched live, and
/// the plugin half resolves this viewer's own activation records — the list is
/// per-viewer, not per-workspace.
#[async_trait]
pub trait SkillCatalog: Send + Sync {
    /// `caller` and `viewer` name the same person; both are needed because
    /// [`ScopeContext::user_id`] is not always a `Uuid`.
    ///
    /// Returns an empty list, not an error, for an unreadable/repository-less
    /// Workspace or any git-host failure — the cause is logged where it
    /// happens.
    async fn list_for_workspace(
        &self,
        viewer: &ScopeContext,
        caller: Uuid,
        workspace_id: &str,
    ) -> Vec<WorkspaceSkill>;
}

/// Why a repository's branches could not be listed.
///
/// [`Self::None`] answers the picker with an empty list; the other two are
/// surfaced or logged where the frame is built.
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
    Failed(moatless_types::store::StoreError),
}

/// A repository's branches, read from its git host.
///
/// The one read here that isn't a database query — needed before any Sandbox
/// exists. `workspace_id` names where the picker opened, so a repository only
/// that workspace's pinned credential can reach still lists.
#[async_trait]
pub trait RepositoryBranchSource: Send + Sync {
    async fn list_branches(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<BranchListResponse, BranchListError>;
}

/// The pull request a Task is bound to, in the shape the status surface
/// reports.
///
/// Base and head are non-optional: a binding predating them yields no
/// `BoundPullRequest` at all, rather than one with blank branches.
#[derive(Clone)]
pub struct BoundPullRequest {
    pub number: u64,
    /// The binding's repository, as `owner/repo`.
    pub repository: String,
    pub title: String,
    pub url: String,
    pub base_ref: String,
    pub head_ref: String,
    /// The binding's normalized state kind: `open`, `draft`, `merged` or
    /// `closed`. The status surface collapses `draft` to `open` for T3's
    /// three-value set.
    pub state_kind: String,
    /// When this deployment last read the pull request's state from GitHub —
    /// not when GitHub last changed it, which no binding records. The client
    /// orders observations of one pull request by it, advancing only when the
    /// status does.
    pub status_read_at: DateTime<Utc>,
}

/// The one of a Task's bound pull requests the singular surfaces report.
///
/// A live pull request outranks a settled one, and the newest number breaks the
/// tie — ordering on the number, not a timestamp, keeps every reader's answer
/// the same.
pub fn primary_pull_request(bound: &[BoundPullRequest]) -> Option<&BoundPullRequest> {
    bound
        .iter()
        .max_by_key(|pr| (is_live_pull_request(&pr.state_kind), pr.number))
}

/// Whether a binding's state kind is one a person can still act on.
fn is_live_pull_request(state_kind: &str) -> bool {
    !matches!(state_kind, "merged" | "closed")
}

/// A binding's state kind, mapped onto T3's three-value set. A draft reports as
/// `open`, because neither `VcsStatusChangeRequestState` nor `PullRequestState`
/// has a `draft`; [`is_draft`] carries the distinction beside the state.
pub(crate) fn t3_pr_state(state_kind: &str) -> &'static str {
    match state_kind {
        "merged" => "merged",
        "closed" => "closed",
        _ => "open",
    }
}

/// Whether a binding's state kind is `draft`, for T3's `isDraft`. Reported
/// beside every state, so a client that keeps a summary does not have to tell a
/// server that omits the field from one saying the pull request left draft.
pub(crate) fn is_draft(state_kind: &str) -> bool {
    state_kind == "draft"
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
/// [`Self::NotFound`] and [`Self::Unavailable`] are surfaced to the dialog
/// verbatim; [`Self::Failed`] is this deployment's own failure, logged rather
/// than sent.
#[derive(Debug)]
pub enum PullRequestError {
    NotFound,
    Unavailable(String),
    Failed(String),
}

/// The pull request a Task is bound to, and resolution of references against a
/// repository's git host.
///
/// A port, not a DAO: resolving a reference reaches the git host, and
/// `crates/t3code` stays free of any GitHub knowledge.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait BoundPullRequestSource: Send + Sync {
    /// Every pull request `task_id` is bound to. Access is checked before this
    /// is called, so it takes no viewer.
    ///
    /// May be several, in no significant order; callers reporting one pick it
    /// with [`primary_pull_request`].
    async fn bound_pull_requests(&self, task_id: &str) -> Vec<BoundPullRequest>;

    /// The Tasks bound to `repository#number`, oldest binding first, for the
    /// linked-thread summary and the guide author.
    ///
    /// Ids only, no side effect: must not resolve or refresh a binding before
    /// the viewer is authorized against these Tasks — narrow first, then
    /// recover via [`Self::bound_pull_requests`].
    async fn tasks_bound_to(&self, repository: &str, number: u64) -> Vec<String>;

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
    /// Task created on a PR's head branch is bound to it. `None` on any
    /// failure, so an unreachable host creates it unbound rather than refused.
    async fn open_pull_request_for_head(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
        head: &str,
    ) -> Option<ResolvedPullRequest>;
}

/// A pull request as read from its git host, for `pullRequests.detail` and a
/// guided review's range.
#[derive(Clone, Debug, Default)]
pub struct PullRequestOverview {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    pub is_draft: bool,
    pub author_login: Option<String>,
    pub author_avatar_url: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub head_ref: String,
    pub head_sha: String,
    /// `owner/repo` of the head branch, when that repository still exists.
    pub head_repository: Option<String>,
    pub base_ref: String,
    pub base_sha: String,
    /// The host's timestamps, RFC 3339.
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub merged_at: Option<String>,
    pub closed_at: Option<String>,
    pub mergeable: Option<bool>,
}

/// One page of a pull request's change, as a `diff --git` patch.
#[derive(Clone, Debug, Default)]
pub struct PullRequestPatchPage {
    pub patch: String,
    /// The page to ask for next, `None` on the last one.
    pub next_page: Option<u32>,
    /// Files on this page the host would not diff, which the patch leaves out.
    pub omitted_files: usize,
}

/// The file [`PullRequestReviewSource::file_contents`] reads.
pub struct ChangedFile<'a> {
    /// A commit of the pull request; `None` is the whole change.
    pub commit: Option<&'a str>,
    pub change: FileChangeType,
    pub old_path: &'a str,
    pub new_path: &'a str,
}

/// What `pullRequests.list` asks of one repository. Every field narrows the
/// listing; the values are the contract's own (`state` `open`, `closed`,
/// `merged` or `all`, `review` `changes-requested`, ...).
#[derive(Clone, Debug, Default)]
pub struct PullRequestListQuery {
    pub state: String,
    /// `authored` or `reviewing`, of `viewer_login`.
    pub involvement: Option<String>,
    /// The viewer's own login on the host, which `involvement` names.
    pub viewer_login: Option<String>,
    pub text: Option<String>,
    pub draft: Option<String>,
    pub review: Option<String>,
    pub checks: Option<String>,
    /// Each group matches any of its labels; every group must match.
    pub labels: Vec<Vec<String>>,
    pub excluded_labels: Vec<String>,
    pub author: Option<String>,
}

/// One pull request in a listing, in the contract's vocabulary.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PullRequestListItem {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author_login: Option<String>,
    pub author_avatar_url: Option<String>,
    pub author_is_bot: bool,
    pub head_ref: String,
    pub base_ref: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    pub is_draft: bool,
    /// `mergeable`, `conflicting` or `unknown`.
    pub mergeability: String,
    pub additions: u64,
    pub deletions: u64,
    /// RFC 3339.
    pub created_at: String,
    pub updated_at: String,
    /// Name and hex colour without `#`.
    pub labels: Vec<(String, Option<String>)>,
    pub review_decision: Option<String>,
    pub checks_state: Option<String>,
    pub requested_reviewers: Vec<String>,
}

/// One page of a repository's listing, newest update first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PullRequestListPage {
    pub items: Vec<PullRequestListItem>,
    /// Opaque; hand back for the next page. `None` on the last one.
    pub next_cursor: Option<String>,
}

/// Pull requests read from their git host, and the Task that writes a guided
/// review.
///
/// Every read takes the Workspace the viewer reached the pull request through,
/// whose credential stands in when the viewer has none of their own; the
/// caller has already checked the viewer may read it.
#[async_trait]
pub trait PullRequestReviewSource: Send + Sync {
    /// `owner/name` of the GitHub repository `workspace_id` lists pull
    /// requests of: its primary one. `None` when the viewer cannot read the
    /// Workspace or its primary repository is not on GitHub.
    async fn project_repository(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, PullRequestError>;

    /// `owner/name` of each GitHub repository a Task in `workspace_id` works
    /// in, over the Tasks `viewer` may read.
    async fn task_repositories(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Vec<String>, PullRequestError>;

    /// Up to `limit` of `repository`'s pull requests matching `query`, from
    /// `cursor` on.
    async fn list(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
        repository: &str,
        query: &PullRequestListQuery,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<PullRequestListPage, PullRequestError>;

    async fn overview(
        &self,
        viewer: &ScopeContext,
        workspace_id: Option<&str>,
        repository: &str,
        number: u64,
    ) -> Result<PullRequestOverview, PullRequestError>;

    /// Page `page` (from 1) of the change.
    async fn patch_page(
        &self,
        viewer: &ScopeContext,
        workspace_id: Option<&str>,
        repository: &str,
        number: u64,
        page: u32,
    ) -> Result<PullRequestPatchPage, PullRequestError>;

    /// Page `page` (from 1) of commit `sha`'s own change.
    async fn commit_patch_page(
        &self,
        viewer: &ScopeContext,
        workspace_id: Option<&str>,
        repository: &str,
        sha: &str,
        page: u32,
    ) -> Result<PullRequestPatchPage, PullRequestError>;

    /// The pull request's conversation, the people on its review and its
    /// newest commits.
    async fn activity(
        &self,
        viewer: &ScopeContext,
        workspace_id: Option<&str>,
        repository: &str,
        number: u64,
    ) -> Result<PullRequestActivity, PullRequestError>;

    /// The page after `cursor` of one review thread's comments. Fails unless
    /// the thread is on `repository#number`.
    async fn thread_comments(
        &self,
        viewer: &ScopeContext,
        workspace_id: Option<&str>,
        repository: &str,
        number: u64,
        thread_id: &str,
        cursor: Option<&str>,
    ) -> Result<ReviewThreadCommentPage, PullRequestError>;

    /// The pull request's labels, requested reviewers and head commit checks.
    async fn status(
        &self,
        viewer: &ScopeContext,
        workspace_id: Option<&str>,
        repository: &str,
        number: u64,
    ) -> Result<PullRequestStatus, PullRequestError>;

    /// One changed file of the pull request, or of `file.commit`, before and
    /// after the change.
    async fn file_contents(
        &self,
        viewer: &ScopeContext,
        workspace_id: Option<&str>,
        repository: &str,
        number: u64,
        file: &ChangedFile<'_>,
    ) -> Result<FileContentsPair, PullRequestError>;

    /// Create a Task in `workspace_id`, owned by `viewer`, whose first message
    /// is `message`, without starting its sandbox. Returns its id.
    async fn create_review_guide_task(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
        name: &str,
        message: String,
    ) -> Result<String, PullRequestError>;

    /// Fork `source_task_id` after its newest settled turn into a Task with
    /// id `task_id`, owned by `viewer`, whose first message is `message`, and
    /// start its sandbox. Returns whether the sandbox started.
    ///
    /// The fork lands in the source's Workspace with its tags, wherever the
    /// viewer opened the pull request: its conversation and checkout are there.
    async fn fork_review_guide_task(
        &self,
        viewer: &ScopeContext,
        source_task_id: &str,
        task_id: &str,
        name: &str,
        message: String,
    ) -> Result<bool, PullRequestError>;
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
/// count and how a stale one is told apart live on the backend side of this
/// boundary.
#[async_trait]
pub trait SourceControlAuthSource: Send + Sync {
    /// The viewer's connected GitHub identity, or `None` when they have no
    /// usable GitHub credential.
    async fn github_identity(&self, viewer: &ScopeContext) -> Option<GithubIdentity>;
}

/// What `attachments.createUploadUrl` was told the file is, carried through the
/// minted URL and re-stated here when the bytes arrive.
///
/// Not what the client said on the upload request itself — these are the values
/// this server validated at mint time and signed into the URL.
pub struct DeclaredAttachment {
    pub filename: String,
    pub mime_type: String,
}

/// Why an attachment's bytes could not be stored or removed.
#[derive(Debug)]
pub enum AttachmentStoreError {
    /// Not this viewer's attachment. Only reachable on delete: the upload route
    /// stores under an id this server minted for the viewer holding the URL.
    Forbidden,
    /// The deployment failed to carry it out. The cause is logged, not sent.
    Failed(String),
}

/// Where a chat attachment's bytes live.
///
/// The id is minted before the bytes move — [`attachments`](crate::attachments)
/// hands the client a URL naming it — so this is `store` under a chosen id, not
/// a create that returns one; S3 and the metadata table stay behind this
/// boundary.
#[async_trait]
pub trait AttachmentStore: Send + Sync {
    /// Store `bytes` under `attachment_id`. The length has already been checked
    /// against what the mint declared.
    async fn store(
        &self,
        viewer: &ScopeContext,
        attachment_id: &str,
        declared: DeclaredAttachment,
        bytes: Vec<u8>,
    ) -> Result<(), AttachmentStoreError>;

    /// Remove an attachment's bytes and its metadata. An id that names nothing
    /// is not a failure: the composer deletes an attachment it abandoned
    /// without knowing whether the upload ever landed.
    async fn delete(
        &self,
        viewer: &ScopeContext,
        attachment_id: &str,
    ) -> Result<(), AttachmentStoreError>;
}

/// Why an Artifact's visibility could not be set.
#[derive(Debug)]
pub enum ArtifactVisibilityError {
    /// No published Artifact by that id on a Task this viewer can read. One
    /// answer for both, so a caller cannot probe for another tenant's pages.
    NotFound,
    /// The viewer can read the Task but not change it.
    Forbidden,
    /// The deployment failed to carry it out. The cause is logged, not sent.
    Failed(String),
}

/// Who may read an Artifact a Task published. A port because Artifacts and
/// the write check on them are the Task band's.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ArtifactVisibilitySetter: Send + Sync {
    async fn set_visibility(
        &self,
        viewer: &ScopeContext,
        artifact_id: &str,
        visibility: Visibility,
    ) -> Result<(), ArtifactVisibilityError>;
}

/// Why a secret request could not be answered.
#[derive(Debug)]
pub enum SecretRequestAnswerError {
    /// No such request on a Task this viewer can read.
    NotFound,
    AlreadyAnswered,
    /// The viewer may not answer it, or sent an answer it cannot take; the
    /// message says which, for the card to show.
    Refused(String),
    /// The deployment failed to store it. The cause is logged, not sent.
    Failed(String),
}

/// Answers a secret request the Task's agent made. A port because the
/// request, the secret store and the wake are the Task band's.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SecretRequestAnswerer: Send + Sync {
    async fn answer(
        &self,
        viewer: &ScopeContext,
        login: &str,
        task_id: &str,
        activity_id: uuid::Uuid,
        answer: moatless_tasks::secret_request::service::Answer,
    ) -> Result<(), SecretRequestAnswerError>;
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

    /// Branch a new Thread off an existing one at one of its completed turns.
    /// Reported the same way `create_thread` is: the client already minted and
    /// navigated to `fork.new_task_id`, so there is nothing to hand back.
    async fn fork(&self, viewer: &ScopeContext, fork: ForkThread) -> Result<(), CommandRefusal>;

    /// Queue a fork's conversation for its parent's next message.
    async fn merge_back(
        &self,
        _viewer: &ScopeContext,
        _source: &str,
        _target: &str,
        _through_turn: i32,
    ) -> Result<(), CommandRefusal> {
        Err(CommandRefusal::Unsupported {
            command: "thread.merge_back".into(),
            reason: "Merge-back is not implemented by this sink.".into(),
        })
    }

    /// Drop every turn of this Thread above `keep_through` (`0` empties it) —
    /// the same cut [`fork`](Self::fork) resolves, applied in place.
    ///
    /// Only the conversation moves; the Sandbox's working copy is left as the
    /// deleted turns wrote it — this serves `thread.conversation.revert`, not
    /// `thread.checkpoint.revert`.
    async fn revert(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        keep_through: i32,
    ) -> Result<(), CommandRefusal>;

    async fn update_meta(
        &self,
        viewer: &ScopeContext,
        meta: ThreadMeta,
    ) -> Result<(), CommandRefusal>;

    /// Stop the turn in flight, leaving the session up: the Sandbox keeps
    /// running, so the next turn starts in the same working copy.
    async fn interrupt(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Stop the session, which in Moatless is stopping the Sandbox. Compute is
    /// released and the working copy is left for the next start; the client
    /// sends this to release a thread, never to end a turn.
    async fn stop(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Set how long this thread's Sandbox may sit idle before Moatless stops
    /// it. `0` disables the idle stop; the value lives on the Task, which is
    /// why it travels here rather than through a sandbox port.
    async fn set_idle_timeout(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        minutes: i32,
    ) -> Result<(), CommandRefusal>;

    /// Set who may reach this thread. `Public` grants every signed-in viewer
    /// read *and* write (see
    /// [`moatless_types::access_control::ScopeContext::can_write`]);
    /// `Private` returns it to its scope and owner's own grants. Refuses
    /// `NotWritable` for anyone but the owner or an admin.
    async fn set_visibility(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        visibility: Visibility,
    ) -> Result<(), CommandRefusal>;

    /// Archive a thread, which in Moatless is closing the Task: the row leaves
    /// the sidebar, the Sandbox is removed, and inbound comments stop waking
    /// it. Children are left open — the cascade is a choice the Moatless UI
    /// asks about.
    async fn archive(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Unarchive a thread, which is reopening the Task. No Sandbox is started;
    /// the next message does that.
    async fn unarchive(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Put this thread in the viewer's own listing (following the Task in
    /// Moatless). Any readable Task may be followed; a re-follow writes
    /// nothing. Grants nothing: decides which listing a Task appears in, never
    /// what a viewer may do with it.
    async fn follow(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Take this thread out of the viewer's own listing. Touches that viewer's
    /// follow and nothing else, so it neither ends the Task nor changes what
    /// anyone else sees.
    async fn unfollow(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    async fn settle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Steer a turn sent with `dispatchMode: "queue"` into the active turn. A
    /// turn the agent has already taken is left as it is.
    async fn steer_queued(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        message_uuid: &str,
    ) -> Result<(), CommandRefusal>;

    async fn unsettle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Change a turn still waiting in the queue, or release a queue a stop
    /// held. A message the agent has already taken is refused, not changed.
    async fn change_queue(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        change: QueueChange,
    ) -> Result<(), CommandRefusal>;

    /// Record that the viewer has seen this thread up to `visited_at`. The
    /// stored mark only moves forward.
    async fn visit(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        visited_at: DateTime<Utc>,
    ) -> Result<(), CommandRefusal>;

    /// Show this thread unread to the viewer again, until their next visit.
    async fn mark_unread(&self, viewer: &ScopeContext, task_id: &str)
    -> Result<(), CommandRefusal>;

    /// Delete the thread and everything it owns. Refuses `NotWritable` for a
    /// viewer who may not delete the Task in the Moatless UI either.
    async fn delete(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        only_if_unused: bool,
    ) -> Result<(), CommandRefusal>;

    async fn snooze(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        snoozed_until: DateTime<Utc>,
    ) -> Result<(), CommandRefusal>;

    async fn unsnooze(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Pin this thread in the viewer's own listing, un-settling it and
    /// clearing its snooze. `order_key` places a fresh pin; a repeated pin
    /// keeps the slot it already has.
    async fn pin(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        order_key: Option<String>,
    ) -> Result<(), CommandRefusal>;

    async fn unpin(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    /// Let the reaper auto-settle this thread for the viewer again, or opt it
    /// out. Neither settles nor unsettles it.
    async fn set_auto_settle(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        enabled: bool,
    ) -> Result<(), CommandRefusal>;

    /// Move this thread within the viewer's pinned block, to the slot the
    /// client's fractional-index `order_key` names.
    async fn set_pin_order_key(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        order_key: String,
    ) -> Result<(), CommandRefusal>;

    /// Move this thread within the viewer's active block, as
    /// [`ThreadCommandSink::set_pin_order_key`] does in the pinned one.
    async fn set_active_order_key(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        order_key: String,
    ) -> Result<(), CommandRefusal>;

    async fn answer_question(
        &self,
        viewer: &ScopeContext,
        answer: QuestionAnswer,
    ) -> Result<(), CommandRefusal>;

    /// Bind a Thread to a pull request, so it appears in `pullRequests` on the
    /// Thread's row. Idempotent: linking one that is already linked reports
    /// success and writes nothing new.
    async fn link_pull_request(
        &self,
        viewer: &ScopeContext,
        reference: ThreadPullRequestRef,
    ) -> Result<(), CommandRefusal>;

    /// Start or stop a linked pull request's events waking the Thread's agent.
    /// Refuses a pull request the Thread has not linked.
    async fn watch_pull_request(
        &self,
        viewer: &ScopeContext,
        reference: ThreadPullRequestRef,
        watching: bool,
    ) -> Result<(), CommandRefusal>;

    /// Release a Thread's pull request, removing the Binding that joined them
    /// so nothing of the join survives a re-link. Idempotent in the same way
    /// [`ThreadCommandSink::link_pull_request`] is.
    async fn unlink_pull_request(
        &self,
        viewer: &ScopeContext,
        reference: ThreadPullRequestRef,
    ) -> Result<(), CommandRefusal>;
}

pub use moatless_tasks::task_turn::usage::{UsageBucket, UsageWindow};

/// Per-model token and cost totals, read from the turns a viewer may see.
///
/// T3's own servers scan the provider CLIs' on-disk transcripts; this
/// deployment has none, so it reads [`moatless_tasks::task_turn::usage::TurnUsage`].
#[async_trait]
pub trait UsageReporter: Send + Sync {
    async fn buckets(
        &self,
        viewer: &ScopeContext,
        window: &UsageWindow,
    ) -> Result<Vec<UsageBucket>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::AttachmentKind;
    use crate::attachments::{MAX_FILE_BYTES, MAX_IMAGE_BYTES};

    /// The member decides the ceiling, and the boundary is where a limit is
    /// either right or off by one. An image is held to the image maximum even
    /// though a file that size would be accepted.
    #[test]
    fn each_member_accepts_up_to_its_own_maximum() {
        assert!(AttachmentKind::Image.accepts("image/png", MAX_IMAGE_BYTES));
        assert!(!AttachmentKind::Image.accepts("image/png", MAX_IMAGE_BYTES + 1));
        assert!(AttachmentKind::File.accepts("application/pdf", MAX_FILE_BYTES));
        assert!(!AttachmentKind::File.accepts("application/pdf", MAX_FILE_BYTES + 1));
        assert!(!AttachmentKind::Image.accepts("image/png", MAX_FILE_BYTES));
    }

    /// The contract's minimum is one byte on both members, so a row recording
    /// an empty or negative size is not something either member can carry.
    #[test]
    fn neither_member_accepts_an_upload_with_no_bytes() {
        assert!(!AttachmentKind::Image.accepts("image/png", 0));
        assert!(!AttachmentKind::File.accepts("application/pdf", 0));
        assert!(!AttachmentKind::File.accepts("application/pdf", -1));
    }

    /// An image is restricted to the four types the mint would have signed for.
    /// A file takes any type — that is the whole difference between the members
    /// — so the check on a file is a size check alone.
    #[test]
    fn only_an_image_is_restricted_by_type() {
        for mime in ["image/gif", "image/jpeg", "image/png", "image/webp"] {
            assert!(AttachmentKind::Image.accepts(mime, 1024), "{mime}");
        }
        // Not one the union names, however plausibly image-shaped.
        assert!(!AttachmentKind::Image.accepts("image/svg+xml", 1024));
        assert!(!AttachmentKind::Image.accepts("application/pdf", 1024));
        assert!(AttachmentKind::File.accepts("application/octet-stream", 1024));
        assert!(AttachmentKind::File.accepts("image/svg+xml", 1024));
    }

    /// The mint lets the file member carry any type up to 50 MiB, image types
    /// included, so a large PNG sent as a file is allowed and accepted here.
    /// `projection::message_attachments` picks the member an attachment fits
    /// for the same reason — reading the mime type instead would silently drop
    /// it from the thread.
    #[test]
    fn the_file_member_takes_an_image_too_large_to_be_one() {
        assert!(AttachmentKind::File.accepts("image/png", MAX_IMAGE_BYTES + 1));
        assert!(AttachmentKind::File.accepts("image/png", MAX_FILE_BYTES));
        assert!(!AttachmentKind::File.accepts("image/png", MAX_FILE_BYTES + 1));
    }

    /// What a row records is what some other route stored, and mime types are
    /// not case-normalised on the way in everywhere.
    #[test]
    fn a_stored_type_is_matched_however_it_was_cased() {
        assert!(AttachmentKind::Image.accepts("IMAGE/PNG", 1024));
        assert!(AttachmentKind::Image.accepts("  image/png  ", 1024));
    }
}
