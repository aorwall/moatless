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
use moatless_types::access_control::ScopeContext;
use moatless_types::store::StoreError;
use moatless_types::task::TaskRelationType;

use moatless_types::agent::AgentMode;

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

/// One attachment the client sent with a turn, in whichever of the two forms
/// the contract lets it take.
///
/// Which form arrives is decided by the `attachmentUploads` capability
/// [`descriptor`](crate::descriptor) reports. A composer that has read it
/// uploads the bytes first and names the id on the turn; one that has not — an
/// older client, or one whose environment config has not loaded yet — puts the
/// bytes in the command as a base64 data URL. Both keep working, so this is a
/// union rather than a replacement.
pub enum TurnAttachment {
    /// Bytes that came in the command itself. This is the decoded result —
    /// [`commands`](super::commands) has already checked that the payload
    /// parses, is an image, and is within the contract's limits, and the mime
    /// type and size are what the bytes actually are rather than what the
    /// client declared them to be.
    Inline {
        /// The name the person's file had, for the transcript and the download.
        filename: String,
        mime_type: String,
        bytes: Vec<u8>,
    },
    /// An upload that already exists, named by the id
    /// `attachments.createUploadUrl` minted for it. Carries no bytes: what the
    /// person attached is in storage, and the name, type and size on the wire
    /// beside this id are the client's copy of what the row already records.
    ///
    /// An id on a turn is only an id. Nothing about it proves it came from a
    /// mint, so `kind` is the member the client claims it under and the
    /// resolver checks the stored row against it — see
    /// [`AttachmentKind::accepts`].
    Uploaded {
        attachment_id: String,
        kind: AttachmentKind,
    },
}

/// Which member of the attachment union a turn claims an upload under.
///
/// The two differ in what they accept, so which one is claimed decides which
/// limits the stored file has to satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    Image,
    File,
}

impl AttachmentKind {
    /// Whether a stored upload could have been minted under this member.
    ///
    /// A turn names an upload by id, and an id is not a provenance: the same
    /// viewer can put a file in storage through Moatless's own multipart route,
    /// which has no notion of T3's union, and then name it here. Resolving it
    /// on read access alone would let that file reach the agent one byte or one
    /// mime type past what `attachments.createUploadUrl` would ever have signed
    /// for. So the row is measured against the member it is claimed under, with
    /// the same numbers the mint uses.
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
    pub agent_mode: Option<AgentMode>,
    /// The reasoning effort selected for this turn, in the agent's own
    /// vocabulary. `None` leaves the agent on its own default.
    pub effort: Option<String>,
    /// The images that came with the turn, in the order the composer held them.
    /// Empty for the overwhelming majority of turns.
    pub attachments: Vec<TurnAttachment>,
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
    /// The first message, when the command carried one. Absent means the Task
    /// is created without a Sandbox.
    pub first_message: Option<ThreadTurn>,
}

/// A title and model change, as this surface understood it.
pub struct ThreadMeta {
    pub task_id: String,
    /// A rename. Absent for the client's auto-title, which this surface drops.
    pub title: Option<String>,
    pub model: Option<String>,
}

/// A fork, as this surface understood it.
pub struct ForkThread {
    /// The id the client minted for the fork and has already navigated to —
    /// the same convention [`NewThread::task_id`] follows.
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
    pub agent_mode: Option<AgentMode>,
    /// The reasoning effort for the first turn, in the agent's own
    /// vocabulary. `None` leaves the agent on its own default.
    pub effort: Option<String>,
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

/// One Task that names another as its parent.
///
/// Carries no access decision: a relation says the edge exists, and whether the
/// child may be shown is settled afterwards by re-reading it through
/// [`ConversationDao::readable_by_ids`](crate::dao::ConversationDao::readable_by_ids).
pub struct ChildTask {
    pub task_id: String,
    /// Which parent edge this is. Both make the child a child; they differ in
    /// what a reader should call it.
    pub relation: TaskRelationType,
}

/// The Tasks that name a Task as their parent.
///
/// A port rather than a query here because relations are the Task band's
/// vocabulary and it already publishes the read (`TaskRelationDao`). Access is
/// deliberately not this port's business — it answers with every edge, and the
/// caller narrows to what the viewer may read.
#[async_trait]
pub trait TaskRelationSource: Send + Sync {
    async fn children(&self, parent_task_id: &str) -> Result<Vec<ChildTask>, StoreError>;
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
    Failed(moatless_backend_core::store::StoreError),
}

/// A repository's branches, read from its git host.
///
/// The one read here that is not a database query: a draft thread's branch
/// picker lists the primary repository's branches before any Sandbox exists,
/// the same list the Moatless UI shows, which means calling out to the git host
/// through the same path Moatless's own `GET /branches` uses.
///
/// `workspace_id` names the workspace the picker was opened in, so a repository
/// only that workspace's pinned credential can reach still lists.
#[async_trait]
pub trait RepositoryBranchSource: Send + Sync {
    async fn list_branches(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
        workspace_id: Option<&str>,
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

/// What `attachments.createUploadUrl` was told the file is, carried through the
/// minted URL and re-stated here when the bytes arrive.
///
/// Not what the client said on the upload request — that request carries no
/// name at all, and its `Content-Type` is whatever it chose to send. These are
/// the values this server validated at mint time and signed into the URL.
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
/// hands the client a URL naming it — so storing is `store` under an id this
/// side chose rather than a create that returns one. `crates/t3code` stays free
/// of S3 and of the upload metadata table; both are behind this boundary.
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
    /// Reported the same way `create_thread` is: the client already minted
    /// and navigated to `fork.new_task_id`, so there is nothing to hand back.
    async fn fork(&self, viewer: &ScopeContext, fork: ForkThread) -> Result<(), CommandRefusal>;

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
}

/// One `(day, provider, model)` cell of usage, already aggregated.
///
/// Token fields are disjoint: `uncached_input_tokens`, `cached_input_tokens`
/// and `cache_creation_tokens` sum to the total input. `reasoning_tokens` is a
/// subset of `output_tokens` and is never added on top.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageBucketRow {
    /// `YYYY-MM-DD` in the time zone the caller asked for.
    pub day: String,
    /// RFC3339 start of the hour, present only for an hourly request.
    pub hour_start: Option<String>,
    /// `claude` or `codex`, as T3's `UsageProviderKind` spells them.
    pub provider: String,
    pub model: String,
    pub uncached_input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    /// What the provider itself reported for these tokens, summed.
    pub cost_usd: f64,
    /// Turns that contributed, which is what a T3 record counts here.
    pub records: i64,
    /// Turns whose tokens are counted and whose cost the provider never
    /// reported.
    pub unpriced_records: i64,
    /// Distinct Tasks, which are this deployment's sessions.
    pub sessions: i64,
}

/// The window a usage read covers, already validated.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageWindow {
    pub since_day: String,
    pub until_day: String,
    /// IANA zone the days are bucketed in. An offset would be wrong for any
    /// window crossing a DST boundary.
    pub time_zone: String,
    /// Bucket by hour instead of by day.
    pub hourly: bool,
}

/// Per-model token and cost totals, read from the turns a viewer may see.
///
/// T3's own servers scan the provider CLIs' on-disk transcripts. This
/// deployment has no such directory: the agent runs in a Sandbox pod that is
/// deleted, so the totals the run reported are the only lasting record and they
/// are already in `task_turns.metrics`.
#[async_trait]
pub trait UsageReporter: Send + Sync {
    async fn buckets(
        &self,
        viewer: &ScopeContext,
        window: &UsageWindow,
    ) -> Result<Vec<UsageBucketRow>, StoreError>;
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
    /// included, so a large PNG sent as a file is something the contract allows
    /// and this side accepts. `projection::message_attachments` picks the member
    /// an attachment fits for the same reason — if it read the member off the
    /// mime type instead, this upload would be stored and read by the agent and
    /// then missing from the thread.
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
