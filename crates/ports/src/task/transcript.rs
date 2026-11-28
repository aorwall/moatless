use async_trait::async_trait;
use moatless_types::store::StoreError;

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
    /// Minted by the caller: the id of the event the row is recorded as.
    pub uuid: String,
    pub text: String,
    pub kind: TranscriptEntryKind,
    /// The Turn this row belongs to — see [`super::TurnCounter`].
    pub turn_number: Option<i32>,
    /// The external event the row became, when an Adapter carried it.
    pub external: Option<ExternalEvent>,
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

/// Where an external event happened: the Adapter, place, and subject.
///
/// An id is unique only within its subject, not across an Adapter, so a lookup
/// states the subject; `task_inbox` scopes by `external_subject_key`,
/// `task_outbound_events` by `source_name` — naming neither is not a match.
#[derive(Debug, Clone, Copy)]
pub struct EventSubject<'a> {
    /// The Adapter that carried the event: `task_inbox.external_adapter` and
    /// `task_outbound_events.provider`.
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
    /// The Turn that wrote it; `None` on a row written before turns existed.
    pub turn_number: Option<i32>,
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

    /// Record an external event this Task produced with no row to hang it on.
    async fn record_external_event(
        &self,
        task_id: &str,
        event: &ExternalEvent,
    ) -> Result<(), StoreError>;

    /// The Task that authored any one of `external_ids` **on `subject`**.
    ///
    /// A caller acts on the answer by binding that Task to the subject; see
    /// [`EventSubject`] for why the ids are only unique within one.
    async fn find_author_on_subject<'a>(
        &self,
        external_ids: &[String],
        subject: EventSubject<'a>,
    ) -> Result<Option<String>, StoreError>;

    /// The Task that authored the comment any one of `external_ids` names,
    /// wherever it wrote it. Suppresses the echo of a reply to its own author.
    ///
    /// Unscoped: correct only because each id is a per-comment provider id
    /// allocated once globally — an id a second subject can repeat belongs in
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
