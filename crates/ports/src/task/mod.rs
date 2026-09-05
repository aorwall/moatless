//! The seams the task domain publishes to peer crates.
//!
//! Loops and the Adapters read a Task to decide what to do with it and write
//! back what they said. Neither needs the `tasks`, `task_messages`,
//! `task_turns`, or `task_tags` rows, so what crosses is the projection each
//! one reads and the entry each one writes. `moatless-tasks` implements every
//! port here.

use async_trait::async_trait;

use chrono::{DateTime, Utc};
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::AgentType;
use moatless_types::task::{RelatedTask, Visibility};
pub use moatless_types::task::{TaskStatus, TurnStatus};

use moatless_types::store::StoreError;

/// The Task facts a peer crate reads. Everything else about a Task — its cost,
/// its run configuration, its sandbox — stays behind the seam.
#[derive(Debug, Clone)]
pub struct TaskDescriptor {
    pub task_id: String,
    pub status: TaskStatus,
    pub name: Option<String>,
    pub owner_user_id: Option<String>,
    /// The Workspace the Task runs in. A peer labelling the Task for a human
    /// resolves this through `WorkspaceResolver`.
    pub workspace_id: Option<String>,
    /// Who started the Task, as a user id. `None` for a Task a Loop fired.
    pub created_by: Option<String>,
}

/// A Task, as the caller may see it.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TaskReader: Send + Sync {
    /// `None` when the Task does not exist or `ctx` may not read it — the same
    /// answer, because a Task a caller cannot read is a Task that is not there.
    async fn describe(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskDescriptor>, StoreError>;

    /// The same predicate as [`Self::describe`], for a caller that needs only
    /// the answer.
    async fn may_access(&self, task_id: &str, ctx: &ScopeContext) -> Result<bool, StoreError>;
}

/// What a transcript row a peer writes is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptEntryKind {
    /// An assistant reply authored in the backend rather than by the agent.
    AssistantReply,
    /// A record of what an Adapter sent outward, so the timeline shows it.
    OutboundStub,
}

/// A transcript row a peer writes on a Task's behalf.
#[derive(Debug, Clone)]
pub struct TranscriptEntry {
    pub task_id: String,
    /// Minted by the caller, so it can tag the row with an external id once the
    /// send that produced it returns.
    pub uuid: String,
    pub text: String,
    pub kind: TranscriptEntryKind,
    /// The Turn this row belongs to — see [`TurnCounter`].
    pub turn_number: Option<i32>,
    pub external_adapter: Option<String>,
    pub external_source_name: Option<String>,
}

/// The external event a transcript row became, or that a Task produced.
#[derive(Debug, Clone)]
pub struct ExternalEvent {
    pub external_id: String,
    /// The Adapter that carried it.
    pub provider: String,
    /// The subject it was carried on — a channel, a thread, an issue.
    pub source_name: Option<String>,
}

/// Where an external event happened: the Adapter that carried it, the
/// provider's own name for the place (`owner/repo` for GitHub), and the
/// subject within that place.
///
/// An external event id is unique only inside a subject, because that is all
/// the provider promises: a pull request number, a per-issue comment index. An
/// Adapter's codec is expected to spell the subject into the id it writes —
/// GitHub's PR-level ids carry `owner/repo#607` — but the tables hold every
/// subject's rows together, so the reader states the subject too. It is then
/// the query, not the codec, that makes the answer mean what it says, and rows
/// written under an earlier codec are placed correctly as well.
///
/// The two tables an event can be recorded in are scoped by what each of them
/// carries: `task_messages` by `external_subject_key`, the exact subject, and
/// `task_outbound_events` by `source_name`, the place. A row that names
/// neither is not a match — a lookup that cannot tell where an event happened
/// must return nothing rather than guess.
#[derive(Debug, Clone, Copy)]
pub struct EventSubject<'a> {
    /// The Adapter that carried the event: `task_messages.external_adapter`
    /// and `task_outbound_events.provider`.
    pub adapter: &'a str,
    /// The provider's own name for where it happened — `owner/repo`.
    pub source_name: &'a str,
    /// The subject within that place — `owner/repo#607`.
    pub subject_key: &'a str,
}

/// Where a transcript row came from, and what it is bound to.
#[derive(Debug, Clone)]
pub struct TranscriptOrigin {
    pub task_id: String,
    pub external_event_id: Option<String>,
    pub external_adapter: Option<String>,
    pub external_subject_key: Option<String>,
}

/// The agent's most recent spoken reply on a Task.
#[derive(Debug, Clone)]
pub struct AssistantReply {
    pub uuid: String,
    pub text: Option<String>,
}

/// One tool call and its result, as the transcript recorded them.
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub result_content: Option<String>,
    pub is_error: Option<bool>,
}

/// A Task's transcript: what was said, and which external events it produced.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TranscriptAppend: Send + Sync {
    /// Write one row. The caller's `uuid` identifies it afterwards.
    async fn append(&self, entry: &TranscriptEntry) -> Result<(), StoreError>;

    /// Tag an already-written row with the external event it became. `false`
    /// when no such row exists.
    async fn set_external_source_metadata(
        &self,
        message_uuid: &str,
        event: &ExternalEvent,
    ) -> Result<bool, StoreError>;

    /// Record an external event this Task produced with no row to hang it on.
    async fn record_external_event(
        &self,
        task_id: &str,
        event: &ExternalEvent,
    ) -> Result<(), StoreError>;

    /// The Task that authored any one of `external_ids` **on `subject`**.
    ///
    /// The answer names a Task as the author of the subject's event, and a
    /// caller acts on it by binding that Task to the subject. Scoping is what
    /// makes the answer mean that: see [`EventSubject`] for the ids that are
    /// only unique within one.
    async fn find_author_on_subject<'a>(
        &self,
        external_ids: &[String],
        subject: EventSubject<'a>,
    ) -> Result<Option<String>, StoreError>;

    /// The Task that authored the comment any one of `external_ids` names,
    /// wherever it wrote it. Suppresses the echo of a reply back to its own
    /// author while sibling Bindings still receive it.
    ///
    /// Unscoped, and correct only because every id it is asked about is a
    /// per-comment provider id — `github:rc:<id>`, `linear:comment:<uuid>` —
    /// that the provider allocates once across all of its subjects. An id
    /// keyed on anything a second subject can repeat belongs in
    /// [`find_author_on_subject`](Self::find_author_on_subject) instead.
    async fn find_comment_author(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, StoreError>;

    /// Every external event id the Task has produced.
    async fn external_event_ids(&self, task_id: &str) -> Result<Vec<String>, StoreError>;

    async fn find_by_uuid(&self, uuid: &str) -> Result<Option<TranscriptOrigin>, StoreError>;

    async fn last_assistant_reply(
        &self,
        task_id: &str,
    ) -> Result<Option<AssistantReply>, StoreError>;

    async fn tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<ToolCallRecord>, StoreError>;
}

/// The Turn a row appended now belongs to.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TurnCounter: Send + Sync {
    async fn latest_turn_number(&self, task_id: &str) -> Result<Option<i32>, StoreError>;
}

/// The Task band's half of [`crate::running::derive_running`], for one Task.
///
/// A peer deciding whether an agent is working holds the Sandbox half already
/// — intent, stop reason, the heartbeat — and is missing only this. The fields
/// carry the rule's own meaning of `None`: *this reader cannot say*, never
/// *this did not happen*.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnSignals {
    pub turn_status: Option<TurnStatus>,
    pub turn_ended_at: Option<DateTime<Utc>>,
    pub latest_user_message_at: Option<DateTime<Utc>>,
}

/// The durable turn state behind the running rule.
///
/// Narrower than [`TurnCounter`] is wide: that port answers "which turn does a
/// row appended now belong to", this one answers "is a turn in flight". They
/// stay apart because the second is read on a status path that must not grow a
/// write-path dependency.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TurnStateReader: Send + Sync {
    async fn turn_signals(&self, task_id: &str) -> Result<TurnSignals, StoreError>;
}

/// Tag names to tag ids.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TagResolver: Send + Sync {
    /// Ids for `names`, creating any tag that does not exist yet.
    async fn resolve(&self, names: &[String]) -> Result<Vec<String>, StoreError>;
}

/// A Task a Loop asks the task band to open.
///
/// The provider path's shape, not the whole creation surface: every Task
/// opened this way is headless, provisions a sandbox, and publishes an `Open`
/// event, so none of those is a field. What varies is the Loop's configuration
/// and what the firing event carried.
#[derive(Debug, Clone)]
pub struct NewTask {
    pub repository_id: String,
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
/// [`Self::Refused`] is the preflight answer — the identity the Task would run
/// as cannot reach what it needs — and is the one variant a caller reports to a
/// person rather than logging.
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

/// Where a turn came from, as the Task's transcript records it.
///
/// `event_id` is what dedup keys on: a provider that re-delivers, or an echo of
/// what this product just sent outward, must not become a second turn.
#[derive(Debug, Clone, Default)]
pub struct TurnOrigin {
    pub event_id: Option<String>,
    pub adapter: Option<String>,
    pub subject_key: Option<String>,
    pub source_name: Option<String>,
    /// The handle the external system knows the writer by, recorded verbatim
    /// and never resolved to a Moatless identity.
    pub author: Option<String>,
    /// The writer's own id in the external system — a Slack `U0000000001`.
    /// What the sender label resolves through, since the Moatless user id on
    /// the send is the subscription's owner when the writer is unmapped.
    pub sender_id: Option<String>,
}

/// A turn arriving from outside the product, addressed to a Task that exists.
///
/// One struct rather than a parameter list: every field below is set
/// per-provider, and the flow it feeds already carried
/// `#[allow(clippy::too_many_arguments)]`.
#[derive(Debug, Clone)]
pub struct InboundTurn {
    pub task_id: String,
    pub text: String,
    pub file_ids: Vec<String>,
    /// Continue the agent's current session rather than starting fresh.
    pub resume: bool,
    pub origin: TurnOrigin,
    /// The identity the turn is attributed to, when the Adapter resolved one.
    pub sent_by: Option<String>,
    /// Skills the sandbox activates for this turn.
    pub skills: Vec<String>,
}

/// Accepting a turn onto an existing Task.
///
/// Distinct from [`TranscriptAppend`], which writes one row and runs no flow: a
/// turn accepted here is deduplicated, persisted, and handed to the agent.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait MessageAcceptor: Send + Sync {
    /// A turn a provider already deduplicated away, and a Task that is closed,
    /// are both `Ok(())` with nothing written — there is no queue to join.
    async fn accept(&self, turn: InboundTurn) -> Result<(), StoreError>;
}

/// Why a provider closed a Task, and what its `Closed` event carries.
///
/// The task band does not read `payload`: the provider owns its shape and the
/// event stream forwards it verbatim.
#[derive(Debug, Clone)]
pub struct CloseReason {
    pub actor: String,
    pub payload: serde_json::Value,
}

/// Closing a Task from the provider path.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TaskFinisher: Send + Sync {
    /// Close `task_id` as the system. A Task that does not exist and a Task the
    /// caller may not control answer alike, so a close never discloses that a
    /// Task is there.
    async fn close(&self, task_id: &str, reason: CloseReason) -> Result<(), StoreError>;
}

/// Recording that one Task relates to another.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait RelationRecorder: Send + Sync {
    /// Best-effort: a relation that cannot be written is logged, never raised.
    /// The Task it would have labelled is already there and usable.
    async fn record_related_to(&self, from_task_id: &str, related: &RelatedTask);
}

/// Which Tasks on a Sandbox are waiting on the agent.
///
/// A Sandbox's wake signal says that work is queued somewhere on it, and
/// several Tasks can share one Sandbox. Delivering that wake means notifying
/// the pod per Task, so the control plane has to name the Tasks the work is
/// actually for. The predicate is a Task-domain fact, so the Task band answers
/// it and the control plane never reads `task_messages`.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait PendingWorkReader: Send + Sync {
    /// Every open Task sharing `task_id`'s Sandbox that holds a user message
    /// the agent has not claimed, `task_id` itself included when it does.
    ///
    /// Empty when the work settled between the wake and this read, which is
    /// ordinary: the caller notifies nobody rather than guessing.
    async fn members_awaiting_dispatch(&self, task_id: &str) -> Result<Vec<String>, StoreError>;
}
