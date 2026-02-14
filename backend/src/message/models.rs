//! Message domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    /// Agent turn result (completed/error/stopped).
    Result,
}

impl MessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::User => "user",
            Self::System => "system",
            Self::Result => "result",
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
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "assistant" => Ok(Self::Assistant),
            "user" => Ok(Self::User),
            "system" => Ok(Self::System),
            "result" => Ok(Self::Result),
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

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for MessageType {
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
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
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

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for MessageRole {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

// =============================================================================
// TurnStatus
// =============================================================================

/// Outcome of an agent turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Completed,
    Error,
    Stopped,
}

impl TurnStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Stopped => "stopped",
        }
    }
}

impl std::fmt::Display for TurnStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for TurnStatus {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "completed" => Ok(Self::Completed),
            "error" => Ok(Self::Error),
            "stopped" => Ok(Self::Stopped),
            other => Err(format!("unknown TurnStatus: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for TurnStatus {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for TurnStatus {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        TurnStatus::try_from(s.as_str()).map_err(Into::into)
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for TurnStatus {
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

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for ContentBlockType {
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
#[allow(dead_code)]
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
    pub attachments: Option<serde_json::Value>,
    pub agent_session_id: Option<String>,
    pub turn_status: Option<TurnStatus>,
    pub turn_result: Option<String>,
    pub will_not_resume: Option<bool>,
    pub run_metrics: Option<serde_json::Value>,
    pub agent_mode: Option<String>,
    pub model: Option<String>,
    pub resume_session_id: Option<String>,
    pub sent_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
}

// =============================================================================
// TaskMessageContext (DB)
// =============================================================================

/// Row from the `task_message_contexts` table.
#[derive(sqlx::FromRow, Debug, Clone)]
#[allow(dead_code)]
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
#[allow(dead_code)]
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

        let parsed: MessageType = serde_json::from_str("\"assistant\"").unwrap();
        assert_eq!(parsed, MessageType::Assistant);
    }

    #[test]
    fn message_role_serde_roundtrip() {
        assert_eq!(serde_json::to_string(&MessageRole::User).unwrap(), "\"user\"");
        assert_eq!(serde_json::to_string(&MessageRole::Assistant).unwrap(), "\"assistant\"");
    }

    #[test]
    fn turn_status_serde_roundtrip() {
        assert_eq!(serde_json::to_string(&TurnStatus::Completed).unwrap(), "\"completed\"");
        assert_eq!(serde_json::to_string(&TurnStatus::Error).unwrap(), "\"error\"");
        assert_eq!(serde_json::to_string(&TurnStatus::Stopped).unwrap(), "\"stopped\"");
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
        assert!(TurnStatus::try_from("pending").is_err());
        assert!(ContentBlockType::try_from("video").is_err());
    }
}
