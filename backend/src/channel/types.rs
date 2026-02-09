//! Channel request/response types and core domain types.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// =========================================================================
// Domain types (not serialized to API)
// =========================================================================

/// Routing mode from channel_configs.routing_mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMode {
    Ongoing,
    PerMessage,
    ThreadBased,
}

impl RoutingMode {
    pub fn parse(s: &str) -> Result<Self, super::error::ChannelError> {
        match s {
            "ongoing" => Ok(Self::Ongoing),
            "per_message" => Ok(Self::PerMessage),
            "thread_based" => Ok(Self::ThreadBased),
            other => Err(super::error::ChannelError::Internal(format!(
                "Unknown routing mode: {other}"
            ))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ongoing => "ongoing",
            Self::PerMessage => "per_message",
            Self::ThreadBased => "thread_based",
        }
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

// =========================================================================
// API request/response types
// =========================================================================

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateChannelConfigRequest {
    pub provider: String,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: String,
    pub prompt_template: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub provider_config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannelConfigRequest {
    pub source_name: Option<String>,
    pub routing_mode: Option<String>,
    pub prompt_template: Option<Option<String>>,
    pub active: Option<bool>,
    #[schema(value_type = Option<Object>)]
    pub provider_config: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfigResponse {
    pub id: String,
    pub provider: String,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: String,
    pub prompt_template: Option<String>,
    pub active: bool,
    #[schema(value_type = Option<Object>)]
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListChannelConfigsQuery {
    pub provider: Option<String>,
    pub repository_id: Option<String>,
}

impl ChannelConfigResponse {
    pub fn from_row(row: &crate::db::models::ChannelConfigRow) -> Self {
        Self {
            id: row.id.clone(),
            provider: row.provider.clone(),
            source_id: row.source_id.clone(),
            source_name: row.source_name.clone(),
            repository_id: row.repository_id.clone(),
            routing_mode: row.routing_mode.clone(),
            prompt_template: row.prompt_template.clone(),
            active: row.active,
            provider_config: row.provider_config.clone(),
            ongoing_task_id: row.ongoing_task_id.clone(),
            created_by: row.created_by.clone(),
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
        }
    }
}
