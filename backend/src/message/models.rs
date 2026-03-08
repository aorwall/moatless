//! Message domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::shared_models::AgentMode;

// =============================================================================
// MessageType
// =============================================================================

/// Type of message in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Assistant,
    User,
    /// SDK user messages stored as "system" to distinguish from frontend user messages.
    System,
    /// Agent turn result (completed/stopped).
    Result,
    /// Agent error result.
    Error,
}

impl MessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::User => "user",
            Self::System => "system",
            Self::Result => "result",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for MessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for MessageType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "assistant" => Ok(Self::Assistant),
            "user" => Ok(Self::User),
            "system" => Ok(Self::System),
            "result" => Ok(Self::Result),
            "error" => Ok(Self::Error),
            other => Err(format!("unknown MessageType: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for MessageType {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for MessageType {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        MessageType::try_from(s.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for MessageType {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

// =============================================================================
// MessageRole
// =============================================================================

/// Role of the message sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
        }
    }
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for MessageRole {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "system" => Ok(Self::System),
            other => Err(format!("unknown MessageRole: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for MessageRole {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for MessageRole {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        MessageRole::try_from(s.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for MessageRole {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

// =============================================================================
// ContentBlockType
// =============================================================================

/// Type of content block within a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContentBlockType {
    Text,
    ToolUse,
    ToolResult,
    Image,
}

impl ContentBlockType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::ToolUse => "tool_use",
            Self::ToolResult => "tool_result",
            Self::Image => "image",
        }
    }
}

impl std::fmt::Display for ContentBlockType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for ContentBlockType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "text" => Ok(Self::Text),
            "tool_use" => Ok(Self::ToolUse),
            "tool_result" => Ok(Self::ToolResult),
            "image" => Ok(Self::Image),
            other => Err(format!("unknown ContentBlockType: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for ContentBlockType {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for ContentBlockType {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        ContentBlockType::try_from(s.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for ContentBlockType {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
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
    pub message_role: MessageRole,
    pub message_content: serde_json::Value,
    pub assistant_message_id: Option<String>,
    pub assistant_message_type: Option<String>,
    pub assistant_message_model: Option<String>,
    pub assistant_stop_reason: Option<String>,
    pub assistant_stop_sequence: Option<String>,
    pub assistant_usage_input_tokens: Option<i32>,
    pub assistant_usage_output_tokens: Option<i32>,
    pub assistant_usage_cache_creation_input_tokens: Option<i32>,
    pub assistant_usage_cache_read_input_tokens: Option<i32>,
    pub tool_use_result: Option<serde_json::Value>,
    pub sdk_metadata: Option<serde_json::Value>,
    pub attachments: Option<serde_json::Value>,
    pub agent_session_id: Option<String>,
    pub will_not_resume: Option<bool>,
    pub run_metrics: Option<serde_json::Value>,
    pub agent_mode: Option<AgentMode>,
    pub model: Option<String>,
    pub resume: bool,
    pub sent_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
}

impl TaskMessage {
    /// Create a new user message with required fields, defaulting all optional fields to `None`.
    pub fn new_user_message(task_id: String, uuid: String, content: serde_json::Value, timestamp: String) -> Self {
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
            message_content: content,
            assistant_message_id: None,
            assistant_message_type: None,
            assistant_message_model: None,
            assistant_stop_reason: None,
            assistant_stop_sequence: None,
            assistant_usage_input_tokens: None,
            assistant_usage_output_tokens: None,
            assistant_usage_cache_creation_input_tokens: None,
            assistant_usage_cache_read_input_tokens: None,
            tool_use_result: None,
            sdk_metadata: None,
            attachments: None,
            agent_session_id: None,
            will_not_resume: None,
            run_metrics: None,
            agent_mode: None,
            model: None,
            resume: true,
            sent_at: None,
            created_at: None,
        }
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

// =============================================================================
// TaskMessageContentBlock (DB)
// =============================================================================

/// Row from the `task_message_content_blocks` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskMessageContentBlock {
    pub id: i32,
    pub task_message_id: i32,
    pub sort_order: i32,
    pub block_type: ContentBlockType,
    pub show_in_ui: bool,
    pub text_content: Option<String>,
    pub tool_use_id: Option<String>,
    pub tool_result_content: Option<String>,
    pub tool_result_is_error: Option<bool>,
    pub image_media_type: Option<String>,
    pub image_data: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_type_serde_roundtrip() {
        assert_eq!(serde_json::to_string(&MessageType::Assistant).unwrap(), "\"assistant\"");
        assert_eq!(serde_json::to_string(&MessageType::User).unwrap(), "\"user\"");
        assert_eq!(serde_json::to_string(&MessageType::System).unwrap(), "\"system\"");
        assert_eq!(serde_json::to_string(&MessageType::Result).unwrap(), "\"result\"");
        assert_eq!(serde_json::to_string(&MessageType::Error).unwrap(), "\"error\"");

        let parsed: MessageType = serde_json::from_str("\"assistant\"").unwrap();
        assert_eq!(parsed, MessageType::Assistant);
    }

    #[test]
    fn message_role_serde_roundtrip() {
        assert_eq!(serde_json::to_string(&MessageRole::User).unwrap(), "\"user\"");
        assert_eq!(serde_json::to_string(&MessageRole::Assistant).unwrap(), "\"assistant\"");
        assert_eq!(serde_json::to_string(&MessageRole::System).unwrap(), "\"system\"");
    }

    #[test]
    fn content_block_type_serde_roundtrip() {
        assert_eq!(serde_json::to_string(&ContentBlockType::Text).unwrap(), "\"text\"");
        assert_eq!(serde_json::to_string(&ContentBlockType::ToolUse).unwrap(), "\"tool_use\"");
        assert_eq!(serde_json::to_string(&ContentBlockType::ToolResult).unwrap(), "\"tool_result\"");
        assert_eq!(serde_json::to_string(&ContentBlockType::Image).unwrap(), "\"image\"");
    }

    #[test]
    fn try_from_unknown_returns_error() {
        assert!(MessageType::try_from("unknown").is_err());
        assert!(MessageRole::try_from("admin").is_err());
        assert!(ContentBlockType::try_from("video").is_err());
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
        assert_eq!(ContentBlockType::Text.to_string(), "text");
        assert_eq!(ContentBlockType::ToolUse.to_string(), "tool_use");
        assert_eq!(ContentBlockType::ToolResult.to_string(), "tool_result");
        assert_eq!(ContentBlockType::Image.to_string(), "image");
    }

    #[test]
    fn new_user_message_sets_defaults() {
        let content = serde_json::json!([{"type": "text", "text": "hello"}]);
        let msg = TaskMessage::new_user_message(
            "task-1".into(),
            "uuid-1".into(),
            content.clone(),
            "2025-01-01T00:00:00.000Z".into(),
        );

        assert_eq!(msg.task_id, "task-1");
        assert_eq!(msg.uuid, "uuid-1");
        assert_eq!(msg.message_type, MessageType::User);
        assert_eq!(msg.message_role, MessageRole::User);
        assert_eq!(msg.user_type, "human");
        assert_eq!(msg.message_content, content);
        assert!(msg.session_id.is_none());
        assert!(msg.agent_session_id.is_none());
    }
}
