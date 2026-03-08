//! Message request/response types for UI consumption.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::message::models::{MessageRole, MessageType};
use crate::shared_models::AgentMode;
use crate::task::schemas::MessageContextInput;

/// UI-facing message representation.
#[derive(Debug, Serialize, ToSchema)]
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
    /// Inline tool results (for assistant messages).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_results: Vec<UiToolResult>,
    /// Whether this assistant message has pending user input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_pending_user_input: Option<bool>,
    /// Assistant-specific metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant: Option<UiAssistantMeta>,
    /// Result-specific metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<UiResultMeta>,
    /// SDK metadata (user flags, assistant extras).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub sdk_metadata: Option<serde_json::Value>,
}

/// Content block within a message.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UiContentBlock {
    #[serde(rename = "type")]
    pub block_type: String, // Stays String: populated from JSON content blocks, not always a known type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object)]
    pub tool_input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_in_ui: Option<bool>,
}

/// Tool result inlined into an assistant message.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UiToolResult {
    pub tool_use_id: String,
    pub content: Option<String>,
    pub is_error: Option<bool>,
    /// Typed tool result from SDK (e.g., BashToolUseResult, ReadToolUseResult).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub typed_result: Option<serde_json::Value>,
}

/// Assistant message metadata.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UiAssistantMeta {
    pub message_id: Option<String>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: Option<UiUsage>,
}

/// Token usage stats.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UiUsage {
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub cache_creation_input_tokens: Option<i32>,
    pub cache_read_input_tokens: Option<i32>,
}

/// Result message metadata.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UiResultMeta {
    pub will_not_resume: Option<bool>,
    #[schema(value_type = Object)]
    pub run_metrics: Option<serde_json::Value>,
}

/// Response for GET /messages.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetMessagesResponse {
    pub messages: Vec<UiMessage>,
    pub total_count: usize,
}

/// A pending message item (user message or result message).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PendingMessageItem {
    pub uuid: String,
    pub message_type: MessageType,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
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
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetPendingMessagesResponse {
    pub messages: Vec<PendingMessageItem>,
    pub total_count: usize,
}

/// Response for DELETE /messages/{id}.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteMessageResponse {
    pub status: String,
}

// =========================================================================
// User input types (AskUserQuestion, ExitPlanMode)
// =========================================================================

/// An option within a question presented to the user.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A question presented to the user during agent interaction.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AllowedPrompt {
    pub tool: String,
    pub prompt: String,
}

/// Input payload for the AskUserQuestion tool.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AskUserQuestionInput {
    pub questions: Vec<Question>,
}

/// Input payload for the ExitPlanMode tool.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    pub message: String,
    pub resume: Option<bool>,
    pub model: Option<String>,
    #[serde(default)]
    pub file_ids: Vec<String>,
    pub command_name: Option<String>,
    pub agent_mode: Option<AgentMode>,
    #[serde(default)]
    pub contexts: Vec<MessageContextInput>,
    // Tool response fields
    pub tool_use_id: Option<String>,
    pub tool_name: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub tool_response: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResponse {
    pub status: String,
    pub timestamp: String,
}
