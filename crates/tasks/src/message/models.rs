//! Message domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};

use moatless_types::activity::Activity;
use moatless_types::agent::AgentMode;

pub use moatless_types::message::{MessageRole, MessageType};

// `stop_is_reported` narrows stop reporting to error stops with the reason
// stated in full: the other stop reasons are routine lifecycle the reader
// already knows from the task's live status and turn state, and for
// externally-bound tasks a "Stopped by user request" line is pure noise on
// Slack or Linear.
//
// A run outcome is nowhere in the conversation, because it is not a Message
// (Decision 14). A stopped turn is known by `task_turns.status`; a Sandbox
// failure by its own activity, which `k8s/pod_watcher.rs:803` already publishes
// with the exit code, terminated reason, node, disruption cause and container
// logs. The Message row would duplicate an event the design stores anyway.

/// `user_type` on a `user` row that exists only to hand a tool response to the
/// agent — the person's answer to `AskUserQuestion` or `ExitPlanMode`.
///
/// The answer renders under the call it settled, not as its own bubble; this
/// names the case explicitly rather than leaving readers to infer and drop the
/// row silently.
pub const TOOL_RESPONSE_USER_TYPE: &str = "tool_response";

// =============================================================================
// Query shapes
// =============================================================================

/// A page of Messages with their context rows and the activities that happened
/// on them — the two ordered collections a conversation is, plus the context
/// rows the frontend attaches to a user Message.
pub type MessagePage = (Vec<TaskMessage>, Vec<TaskMessageContext>, Vec<Activity>);

/// How much of the parent a fork's conversation copy carried.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ForkCopyCounts {
    pub messages: usize,
    pub activities: usize,
    pub contexts: usize,
}

/// Raw tool-call row assembled from the assistant `tool_use` block plus any
/// matching `tool_result` row(s). Keeps JSON values un-transformed so the
/// handler can decide which schema to apply.
#[derive(Debug, Clone)]
pub struct ToolCallRow {
    pub tool_use_id: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub result_content: Option<String>,
    pub is_error: Option<bool>,
    pub tool_use_result: Option<serde_json::Value>,
    /// UUID of the assistant row containing the `tool_use` block. Surfaced so
    /// post-message hooks can tag the parent message rather than the hidden
    /// System `tool_result` row.
    pub parent_message_uuid: String,
}

/// One Task carrying unacknowledged user messages, the desired-state
/// reconciler's message-wake input.
///
/// A user-turn acknowledged by the sandbox but never reaching a terminal
/// `agent.completed/error/stopped` event (i.e. the sandbox crashed/restarted
/// mid-run); drives a `claude --resume <session_id>` replay on startup.
#[derive(Debug, Clone)]
pub struct InterruptedTurn {
    /// `task_messages.id` of the turn's first user row, for logging. The
    /// recovery counter lives on `task_turns` — a turn can hold several user
    /// messages, so bumping one of them would undercount the batch.
    pub task_message_id: i32,
    pub task_id: String,
    /// Backend-tracked agent session ID. Either `session_id` or
    /// `agent_session_id` — they're set together at acknowledgement.
    pub session_id: String,
    pub turn_number: i32,
    pub prompt_text: String,
    pub agent_mode: Option<String>,
    /// Reasoning effort the interrupted turn ran with, so the replay repeats
    /// it.
    pub effort: Option<String>,
    pub recovery_attempts: i32,
    /// The turn's original `run_id`, so the recovery re-drive can reuse it and
    /// keep its events on this turn. `None` for legacy turns with no recorded
    /// `run_id` — those fall back to a fresh run (the pre-existing behaviour).
    pub run_id: Option<String>,
}

// =============================================================================
// TaskMessage (DB)
// =============================================================================

/// Row from the `task_messages` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskMessage {
    pub id: i32,
    pub task_id: String,
    pub uuid: String,
    #[sqlx(try_from = "String")]
    pub message_type: MessageType,
    pub user_type: String,
    pub parent_uuid: Option<String>,
    pub is_sidechain: Option<bool>,
    pub cwd: Option<String>,
    pub session_id: Option<String>,
    pub turn_number: Option<i32>,
    pub version: Option<String>,
    pub git_branch: Option<String>,
    pub request_id: Option<String>,
    pub timestamp: String,
    #[sqlx(try_from = "String")]
    pub message_role: MessageRole,
    /// What was said, as text. `None` for a Message with no text — which after
    /// the conversation split is a row that should not exist, and does only
    /// where something wrote one before its text arrived.
    pub message_text: Option<String>,
    pub assistant_message_id: Option<String>,
    pub assistant_message_type: Option<String>,
    pub assistant_message_model: Option<String>,
    pub assistant_stop_reason: Option<String>,
    pub assistant_stop_sequence: Option<String>,
    pub assistant_usage_input_tokens: Option<i32>,
    pub assistant_usage_output_tokens: Option<i32>,
    pub assistant_usage_cache_creation_input_tokens: Option<i32>,
    pub assistant_usage_cache_read_input_tokens: Option<i32>,
    /// Complete raw token-usage object from the assistant message (tokens,
    /// cache-write breakdown, cache reads, server tool use, service tier).
    ///
    /// Kept whole so cost can be derived downstream from tokens + model — the
    /// tmux transport never reports `total_cost_usd`.
    pub assistant_usage: Option<serde_json::Value>,
    pub tool_use_result: Option<serde_json::Value>,
    /// A person's answer to an interactive tool call, structure intact:
    /// `{toolUseId, toolName, answers?, approved?, feedback?}`.
    ///
    /// Reaches the sandbox as a prompt response, not text. `None` unless this
    /// row is a tool response.
    pub tool_response: Option<serde_json::Value>,
    pub sdk_metadata: Option<serde_json::Value>,
    pub attachments: Option<serde_json::Value>,
    pub agent_session_id: Option<String>,
    pub will_not_resume: Option<bool>,
    pub run_metrics: Option<serde_json::Value>,
    /// Persisted column. Read the typed value through
    /// [`TaskMessage::agent_mode`].
    #[sqlx(rename = "agent_mode")]
    pub agent_mode_raw: Option<String>,
    pub model: Option<String>,
    /// Reasoning effort for the turn, in the agent's own vocabulary. `None`
    /// leaves the agent on its own default.
    pub effort: Option<String>,
    /// Skills this send activates. Names only; the sandbox resolves each one's
    /// `SKILL.md` on disk and frontloads its body ahead of the prompt text.
    pub skills: Vec<String>,
    pub resume: bool,
    pub sent_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub external_event_id: Option<String>,
    pub external_adapter: Option<String>,
    /// Thread/binding identity this inbound message belongs to (the binding's
    /// `subject_key`, e.g. `T06…:C0A…:1781593153.718929`).
    ///
    /// Lets a reply select the exact binding and lets the agent tell concurrent
    /// threads apart. `None` for non-binding-sourced rows.
    pub external_subject_key: Option<String>,
    pub external_source_name: Option<String>,
    /// The handle the external system knows the writer by — a GitHub login, a
    /// Slack display name, a Linear actor.
    ///
    /// Not a Moatless identity and never resolved to one: `sent_by` holds that,
    /// and on most adapters names the subscription's owner, not whoever wrote
    /// this.
    pub external_author: Option<String>,
    /// The raw external id of whoever wrote this — a Slack `U0000000001`.
    ///
    /// The sender label resolves from this, never `sent_by`, since `sent_by`
    /// holds the trigger owner when the writer has no Moatless mapping. `None`
    /// on internal rows and rows predating this column.
    pub external_sender_id: Option<String>,
    pub source_task_id: Option<String>,
    /// User declared the next agent run should NOT resume the prior session.
    /// Distinct from `will_not_resume`, which lives on the result row.
    pub start_fresh: bool,
    /// Moatless user id of the sender. `None` for system-/adapter-originated
    /// rows (schedulers, bindings, external webhooks, error results).
    pub sent_by: Option<String>,
}

impl TaskMessage {
    /// The agent mode this message was produced under, when the column names
    /// one.
    pub fn agent_mode(&self) -> Option<AgentMode> {
        self.agent_mode_raw.clone().map(AgentMode::from)
    }

    /// The queued user row every send path starts from: no turn, no session,
    /// nothing acknowledged, and no origin. The run that consumes it decides
    /// which turn it belongs to, and each caller fills in what its own origin
    /// adds. Every optional field defaults to `None`; `resume` defaults to
    /// `true`, so a caller queueing a row that must not resume sets it.
    pub fn new_user_message(
        task_id: String,
        uuid: String,
        text: String,
        timestamp: String,
    ) -> Self {
        Self {
            id: 0,
            task_id,
            uuid,
            message_type: MessageType::User,
            user_type: "human".to_string(),
            parent_uuid: None,
            is_sidechain: None,
            cwd: None,
            session_id: None,
            turn_number: None,
            version: None,
            git_branch: None,
            request_id: None,
            timestamp,
            message_role: MessageRole::User,
            message_text: Some(text),
            assistant_message_id: None,
            assistant_message_type: None,
            assistant_message_model: None,
            assistant_stop_reason: None,
            assistant_stop_sequence: None,
            assistant_usage_input_tokens: None,
            assistant_usage_output_tokens: None,
            assistant_usage_cache_creation_input_tokens: None,
            assistant_usage_cache_read_input_tokens: None,
            assistant_usage: None,
            tool_use_result: None,
            tool_response: None,
            sdk_metadata: None,
            attachments: None,
            agent_session_id: None,
            will_not_resume: None,
            run_metrics: None,
            agent_mode_raw: None,
            model: None,
            effort: None,
            skills: Vec::new(),
            resume: true,
            sent_at: None,
            created_at: None,
            external_event_id: None,
            external_adapter: None,
            external_subject_key: None,
            external_source_name: None,
            external_author: None,
            external_sender_id: None,
            source_task_id: None,
            start_fresh: false,
            sent_by: None,
        }
    }

    /// Extract the persisted file IDs from the `attachments` JSON column.
    ///
    /// Format: `{"file_ids": ["<uuid>", ...]}`; empty when absent or missing
    /// that array.
    pub fn file_ids(&self) -> Vec<String> {
        self.attachments
            .as_ref()
            .and_then(|v| v.get("file_ids"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }
}

// =============================================================================
// TaskMessageContext (DB)
// =============================================================================

/// Row from the `task_message_contexts` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskMessageContext {
    pub id: i32,
    pub task_message_id: i32,
    #[sqlx(rename = "type")]
    pub context_type: String,
    pub reference_id: String,
    pub data: serde_json::Value,
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_type_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&MessageType::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(
            serde_json::to_string(&MessageType::User).unwrap(),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&MessageType::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            serde_json::to_string(&MessageType::Result).unwrap(),
            "\"result\""
        );
        assert_eq!(
            serde_json::to_string(&MessageType::Error).unwrap(),
            "\"error\""
        );

        let parsed: MessageType = serde_json::from_str("\"assistant\"").unwrap();
        assert_eq!(parsed, MessageType::Assistant);
    }

    #[test]
    fn message_role_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&MessageRole::User).unwrap(),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::System).unwrap(),
            "\"system\""
        );
    }

    #[test]
    fn try_from_unknown_returns_error() {
        assert!(MessageType::try_from("unknown").is_err());
        assert!(MessageRole::try_from("admin").is_err());
    }

    #[test]
    fn display_impls() {
        assert_eq!(MessageType::User.to_string(), "user");
        assert_eq!(MessageType::Assistant.to_string(), "assistant");
        assert_eq!(MessageType::System.to_string(), "system");
        assert_eq!(MessageType::Result.to_string(), "result");
        assert_eq!(MessageType::Error.to_string(), "error");
        assert_eq!(MessageRole::User.to_string(), "user");
        assert_eq!(MessageRole::Assistant.to_string(), "assistant");
        assert_eq!(MessageRole::System.to_string(), "system");
    }

    #[test]
    fn new_user_message_sets_defaults() {
        let msg = TaskMessage::new_user_message(
            "task-1".into(),
            "uuid-1".into(),
            "hello".into(),
            "2025-01-01T00:00:00.000Z".into(),
        );

        assert_eq!(msg.task_id, "task-1");
        assert_eq!(msg.uuid, "uuid-1");
        assert_eq!(msg.message_type, MessageType::User);
        assert_eq!(msg.message_role, MessageRole::User);
        assert_eq!(msg.user_type, "human");
        assert_eq!(msg.message_text.as_deref(), Some("hello"));
        assert!(msg.session_id.is_none());
        assert!(msg.agent_session_id.is_none());
    }

    #[test]
    fn file_ids_returns_empty_when_attachments_absent() {
        let msg = TaskMessage::new_user_message(
            "task-1".into(),
            "uuid-1".into(),
            String::new(),
            "ts".into(),
        );
        assert!(msg.file_ids().is_empty());
    }

    #[test]
    fn file_ids_extracts_array_from_attachments() {
        let mut msg = TaskMessage::new_user_message(
            "task-1".into(),
            "uuid-1".into(),
            String::new(),
            "ts".into(),
        );
        msg.attachments = Some(serde_json::json!({ "file_ids": ["a", "b", "c"] }));
        assert_eq!(
            msg.file_ids(),
            vec!["a".to_string(), "b".into(), "c".into()]
        );
    }

    #[test]
    fn file_ids_skips_non_string_entries() {
        let mut msg = TaskMessage::new_user_message(
            "task-1".into(),
            "uuid-1".into(),
            String::new(),
            "ts".into(),
        );
        msg.attachments = Some(serde_json::json!({ "file_ids": ["a", 42, null, "b"] }));
        assert_eq!(msg.file_ids(), vec!["a".to_string(), "b".into()]);
    }
}
