//! The seams the task domain publishes to peer crates.
//!
//! Loops and the Adapters read a Task to decide what to do with it and write
//! back what they said. Neither needs the `tasks`, `task_messages`,
//! `task_turns`, or `task_tags` rows, so what crosses is the projection each
//! one reads and the entry each one writes. `moatless-tasks` implements all
//! four ports.

use async_trait::async_trait;

use moatless_types::access_control::ScopeContext;
pub use moatless_types::task::TaskStatus;

use crate::store::StoreError;

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

    /// The Task that authored any one of `external_ids`. Suppresses the echo of
    /// an event back to its own author while sibling Bindings still receive it.
    async fn find_by_external_event_id(
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

/// Tag names to tag ids.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TagResolver: Send + Sync {
    /// Ids for `names`, creating any tag that does not exist yet.
    async fn resolve(&self, names: &[String]) -> Result<Vec<String>, StoreError>;
}
