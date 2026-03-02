//! Channel domain types — DB row structs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// =============================================================================
// ChannelConfig (DB)
// =============================================================================

/// Row from the `channel_configs` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ChannelConfig {
    pub id: String,
    pub provider: String,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    pub active: bool,
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub response_config_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// ChannelThread (DB)
// =============================================================================

/// Row from the `channel_threads` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ChannelThread {
    pub id: i32,
    pub config_id: String,
    pub task_id: String,
    pub external_thread_id: String,
    pub properties: serde_json::Value,
    pub last_message_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

// =============================================================================
// Domain types (internal, not serialized to API)
// =============================================================================

/// Routing mode from channel_configs.routing_mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    Ongoing,
    PerMessage,
    ThreadBased,
}

impl RoutingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ongoing => "ongoing",
            Self::PerMessage => "per_message",
            Self::ThreadBased => "thread_based",
        }
    }
}

impl std::fmt::Display for RoutingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for RoutingMode {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "ongoing" => Ok(Self::Ongoing),
            "per_message" => Ok(Self::PerMessage),
            "thread_based" => Ok(Self::ThreadBased),
            other => Err(format!("unknown RoutingMode: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for RoutingMode {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for RoutingMode {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        RoutingMode::try_from(s.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for RoutingMode {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

/// Inbound message produced by a provider-specific parser.
#[derive(Debug, Clone)]
pub struct InboundMessage {
    /// Provider type.
    pub provider: super::provider::ChannelType,
    /// Source identifier (Slack channelId, email address).
    pub source_id: String,
    /// Thread ID if this is a reply (Slack thread_ts, Email In-Reply-To).
    pub thread_id: Option<String>,
    /// New thread ID for the message itself (Slack message ts, Email Message-ID).
    pub message_thread_id: String,
    /// Resolved user ID (system user).
    pub user_id: String,
    /// Message text content.
    pub text: String,
    /// File attachment IDs (already uploaded to S3).
    pub file_ids: Vec<String>,
    /// Provider-specific properties to store on thread.
    pub thread_properties: serde_json::Value,
    /// Provider-specific context.
    pub provider_context: serde_json::Value,
}

/// Outbound reply to send back to a channel.
#[derive(Debug)]
pub struct OutboundReply {
    pub text: String,
    pub task_url: Option<String>,
    pub is_error: bool,
}

/// Status indicator for a message (e.g., Slack emoji reactions).
#[derive(Debug, Clone, Copy)]
pub enum MessageStatus {
    Processing,
    Completed,
    Error,
    PendingResponse,
}

impl MessageStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::PendingResponse => "pending",
        }
    }
}
