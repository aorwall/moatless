//! Message request/response types for UI consumption.

use serde::{Deserialize, Serialize};

use crate::enums::{MessageRole, MessageType};
use crate::shared::AgentMode;
use crate::task::MessageContextInput;

/// UI-facing message representation.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UiMessage {
    pub uuid: String,
    pub message_type: MessageType,
    pub role: MessageRole,
    pub timestamp: String,
    pub session_id: Option<String>,
    pub turn_number: Option<i32>,
    pub agent_mode: Option<AgentMode>,
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_queued: Option<bool>,
    /// Content blocks for this message.
    pub content: Vec<UiContentBlock>,
    /// Contexts attached to this user message (file uploads, preview URLs,
    /// selected elements).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contexts: Vec<MessageContextInput>,
    /// Inline tool results (for assistant messages).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_results: Vec<UiToolResult>,
    /// Whether this assistant message has pending user input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_pending_user_input: Option<bool>,
    /// Provider message id for assistant rows (e.g. `msg_abc...`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// Provider stop reason for assistant rows (e.g. `end_turn`, `tool_use`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// Provider stop sequence for assistant rows, when one was hit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>,
    /// Token usage stats for assistant rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UiUsage>,
    /// Result-specific metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<UiResultMeta>,
    /// SDK metadata (user flags, assistant extras).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub sdk_metadata: Option<serde_json::Value>,
    /// External source metadata for messages originating from adapters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_source: Option<UiMessageExternalSource>,
    /// Parent tool use ID (links subagent messages to parent Agent tool call).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_uuid: Option<String>,
    /// Task ID that sent this message (present for inter-task messages only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_task_id: Option<String>,
    /// Moatless user id that authored this message. Absent for system- or
    /// adapter-originated rows (schedulers, bindings, external webhooks,
    /// assistant turns, error results).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_by: Option<String>,
}

/// External source metadata for messages originating from adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UiMessageExternalSource {
    /// Adapter type (e.g. "slack", "webhook", "github", "linear").
    pub adapter_kind: String,
    /// Human-readable source name (e.g. "#engineering").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    /// Stable, type-namespaced external id (e.g. "github:ic:12345",
    /// "slack:ts:1700000.000123"). Present when the row was tagged by an
    /// inbound webhook or by the outbound dedup fix; absent for legacy
    /// rows written before the prefix scheme.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Human-readable rendering of `external_id` (e.g. "Comment #12345",
    /// "PR #42 opened", "ts 1700000.000123"). Falls back to the raw
    /// `external_id` for legacy or unknown prefix shapes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_label: Option<String>,
}

/// Status of a spawned subagent, derived from the outer `Agent` tool_result:
/// present + `is_error` → `Error`; present → `Completed`; absent → `Running`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub enum SubagentStatus {
    Running,
    Completed,
    Error,
}

/// Derived aggregate metadata for a spawned subagent, carried on the `Agent`
/// tool-use content block. Not content — the collapsed `Agent` card renders as
/// a pure read of this. Shared shape between the read path and the live
/// `subagentProgress` event (absolute values).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SubagentSummary {
    /// `COUNT(*)` of the subagent's rows (rows whose `parent_uuid` equals the
    /// `Agent` tool-use id).
    pub message_count: u32,
    /// `SUM(input_tokens + output_tokens)` across the subagent's assistant rows.
    pub tokens: u32,
    pub status: SubagentStatus,
}

/// Content block within a message.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UiContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub tool_input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_in_ui: Option<bool>,
    /// For `Agent` tool-use blocks only: a derived summary of the spawned
    /// subagent's activity. Injected post-transform from a batched aggregate;
    /// absent on non-`Agent` blocks and on the live `subagentProgress` path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_summary: Option<SubagentSummary>,
}

/// Tool result inlined into an assistant message.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UiToolResult {
    pub tool_use_id: String,
    pub content: Option<String>,
    pub is_error: Option<bool>,
    /// Typed tool result from SDK (e.g., BashToolUseResult, ReadToolUseResult).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub typed_result: Option<serde_json::Value>,
}

/// Token usage stats.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UiUsage {
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub cache_creation_input_tokens: Option<i32>,
    pub cache_read_input_tokens: Option<i32>,
}

/// Result message metadata.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UiResultMeta {
    pub will_not_resume: Option<bool>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub run_metrics: Option<serde_json::Value>,
}

/// Response for GET /messages.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GetMessagesResponse {
    pub messages: Vec<UiMessage>,
    /// Cursor for fetching the next older page; `None` when no older rows remain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_before_id: Option<i32>,
    /// Whether older messages exist beyond the returned page.
    pub has_more: bool,
    /// Number of messages in this response. Required by the upstream
    /// moatless-swarm-ui CLI which deserializes against a `totalCount: usize`
    /// field; omitting it breaks `moat messages list <task_id>`.
    pub total_count: usize,
}

/// A pending message item (user message or result message).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PendingMessageItem {
    pub uuid: String,
    pub message_type: MessageType,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub message_content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<AgentMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub resume: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub will_not_resume: Option<bool>,
}

/// Response for GET /messages/pending.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GetPendingMessagesResponse {
    pub messages: Vec<PendingMessageItem>,
    pub total_count: usize,
}

/// Full tool-call detail returned on demand (when a tool call is expanded).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ToolCallDetailResponse {
    pub tool_use_id: String,
    pub tool_name: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub tool_input: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub typed_result: Option<serde_json::Value>,
}

/// Response for GET /tasks/{task_id}/tool-calls/{tool_use_id}/messages — the
/// on-demand subagent transcript. The assembled, fold-resolved Messages of the
/// subagent spawned by the given `Agent` tool call.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SubagentTranscriptResponse {
    pub messages: Vec<UiMessage>,
}

// =========================================================================
// User input types (AskUserQuestion, ExitPlanMode)
// =========================================================================

/// An option within a question presented to the user.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A question presented to the user during agent interaction.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
}

/// A tool/prompt that the agent requests permission to execute.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AllowedPrompt {
    pub tool: String,
    pub prompt: String,
}

/// Input payload for the AskUserQuestion tool.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AskUserQuestionInput {
    pub questions: Vec<Question>,
}

/// Input payload for the ExitPlanMode tool.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ExitPlanModeInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_prompts: Option<Vec<AllowedPrompt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

// =========================================================================
// Send message / command types
// =========================================================================

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    pub message: String,
    pub resume: Option<bool>,
    pub model: Option<String>,
    #[serde(default)]
    pub file_ids: Vec<String>,
    pub agent_mode: Option<AgentMode>,
    #[serde(default)]
    pub contexts: Vec<MessageContextInput>,
    // Tool response fields
    pub tool_use_id: Option<String>,
    pub tool_name: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub tool_response: Option<serde_json::Value>,
    /// Task ID of the sender (set automatically when sent from within a sandbox).
    pub source_task_id: Option<String>,
    /// When true, the next agent run starts a fresh session instead of resuming.
    /// Per-message and cleared after send.
    pub start_fresh: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResponse {
    pub status: String,
    pub timestamp: String,
    /// Persisted `uuid` of the user Message, so the client can reconcile its
    /// optimistic entry with the later `message.upsert`. `None` on the
    /// tool-response branch, which produces no single user Message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

// =========================================================================
// Reactions
// =========================================================================

/// Request body for POST /api/v1/messages/{message_id}/reactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AddMessageReactionRequest {
    /// Emoji name (e.g. "thumbsup", "+1", "rocket"). Platform-native names accepted.
    pub emoji: String,
}

/// Response from reaction add/remove endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct MessageReactionResponse {
    pub ok: bool,
    /// Provider-specific reaction id, when the API returns one
    /// (e.g. GitHub reaction id, Linear reaction UUID).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}

// =========================================================================
// Replies to individual messages
// =========================================================================

/// Request body for POST /api/v1/messages/{message_id}/reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReplyToMessageRequest {
    pub text: String,
    #[serde(default)]
    pub file_ids: Vec<String>,
}

/// Response from the reply endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReplyToMessageResponse {
    pub ok: bool,
    /// Provider-specific external id of the reply we posted, when the API returns one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}
