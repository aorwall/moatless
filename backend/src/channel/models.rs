//! Channel domain types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::scope::Scope;

/// Persisted `task_bindings.origin` value for bindings created from inbound
/// provider events. The stored value remains `ingress` for existing database
/// rows and API compatibility.
pub const PROVIDER_EVENT_BINDING_ORIGIN: &str = "ingress";
pub const CONNECTED_THREAD_BINDING_ORIGIN: &str = "connected_thread";

// =============================================================================
// Provider contexts
// =============================================================================

/// Provider-facing route context.
///
/// This is synthesized from canonical routes, triggers, bindings, or scheduled
/// jobs before invoking provider-specific delivery/parsing code.
#[derive(Debug, Clone)]
pub struct TriggerContext {
    pub id: String,
    pub provider: String,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    pub active: bool,
    pub allow_direct_send: bool,
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TriggerContext {
    /// The user ID to attribute tasks to when the actual sender cannot be resolved.
    /// Falls back to `created_by` when `run_as_user_id` is not set.
    pub fn effective_user_id(&self) -> &str {
        self.run_as_user_id.as_deref().unwrap_or(&self.created_by)
    }
}

// =============================================================================
/// Provider-facing subject context.
///
/// This is synthesized from a task binding before invoking provider-specific
/// outbound and context-fetching code.
#[derive(Debug, Clone)]
pub struct BindingContext {
    pub binding_id: String,
    pub trigger_id: Option<String>,
    pub task_id: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub destination_config: serde_json::Value,
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    pub last_message_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

// =============================================================================
// Target architecture tables (DB)
// =============================================================================

/// Row from the `provider_connections` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ProviderConnection {
    pub id: String,
    pub provider: String,
    pub connection_kind: String,
    pub scope_value: String,
    pub credential_ref: Option<String>,
    pub webhook_secret: Option<String>,
    /// Provider-specific transport metadata (JSONB). Each provider stores its
    /// own flat bag of identifiers and config here so we don't need
    /// per-provider columns on this table.
    ///
    /// Conventions in use today:
    /// - `source`: `"env"` | `"oauth"` — where the credential came from.
    /// - `slack_workspace_id`: e.g. `"T0000000002"` — the Slack workspace ID.
    ///   Used to build `task_bindings.subject_key` for DM replies so that
    ///   inbound webhook events route back to the originating task.
    pub transport_config: serde_json::Value,
    pub created_by: String,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Row from the `task_routes` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskRoute {
    pub id: String,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TaskRoute {
    /// The user ID to attribute tasks to when the actual sender cannot be resolved.
    /// Falls back to `created_by` when `run_as_user_id` is not set.
    pub fn effective_user_id(&self) -> &str {
        self.run_as_user_id.as_deref().unwrap_or(&self.created_by)
    }
}

/// Row from the `task_triggers` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskTrigger {
    pub id: String,
    pub provider: String,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub event_filter: serde_json::Value,
    pub trigger_config: Option<serde_json::Value>,
    pub route_id: String,
    pub allow_direct_send: bool,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
    pub active: bool,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Canonical trigger+route view backed by `task_triggers JOIN task_routes`.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskTriggerRoute {
    pub id: String,
    pub provider: String,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub routing_mode: RoutingMode,
    pub event_filter: serde_json::Value,
    pub repository_id: String,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub allow_direct_send: bool,
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
    pub active: bool,
    pub created_by: String,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TaskTriggerRoute {
    /// The user ID to attribute tasks to when the actual sender cannot be resolved.
    /// Falls back to `created_by` when `run_as_user_id` is not set.
    pub fn effective_user_id(&self) -> &str {
        self.run_as_user_id.as_deref().unwrap_or(&self.created_by)
    }
}

/// Row from the `task_bindings` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskBinding {
    pub id: String,
    pub task_id: String,
    pub provider: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub connection_id: String,
    pub trigger_id: Option<String>,
    pub destination_config: serde_json::Value,
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    pub origin: String,
    pub created_at: DateTime<Utc>,
    pub last_event_at: DateTime<Utc>,
}

/// Row from the `scheduled_jobs` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ScheduledJob {
    pub id: String,
    pub route_id: String,
    pub schedule_expr: String,
    pub timezone: String,
    pub message_template: String,
    pub run_as_user_id: Option<String>,
    pub active: bool,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub next_fire_at: DateTime<Utc>,
    pub locked_by: Option<String>,
    pub locked_until: Option<DateTime<Utc>>,
    pub created_by: String,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// Domain types (internal, not serialized to API)
// =============================================================================

pub use moatless_api_schemas::enums::RoutingMode;

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
    /// External message ID for dedup (e.g., Slack `ts`, Telegram `message_id`).
    /// Must be unique per message — NOT the thread ID.
    pub channel_external_id: Option<String>,
}

/// Resolved file attachment for outbound channel messages.
#[derive(Clone, Debug)]
pub struct OutboundFile {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

/// Outbound reply to send back to a channel.
#[derive(Debug)]
pub struct OutboundReply {
    pub text: String,
    pub task_url: Option<String>,
    pub is_error: bool,
    pub files: Vec<OutboundFile>,
}

/// Info returned by `send_message` to create a new `BindingContext`.
#[derive(Debug)]
pub struct NewThreadInfo {
    /// Provider-specific thread/message ID (e.g., Slack message `ts`).
    pub external_thread_id: String,
    /// Provider-specific properties for the thread (e.g., `channel`, `thread_ts`).
    pub properties: serde_json::Value,
}

// =============================================================================
// Shared access-control types (used by Telegram, Slack, and future providers)
// =============================================================================

/// Access policy for a channel (DM, group, or Slack channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChannelAccessPolicy {
    /// Accept messages from any source.
    #[default]
    Open,
    /// Reject all messages.
    Disabled,
    /// Accept only from sources in the allowlist.
    Allowlist,
    /// Allowlist + prompt unknown users with a pairing code (DM only).
    Pairing,
}

/// When to trigger processing of an inbound message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTrigger {
    /// Only process messages that mention the bot.
    #[default]
    MentionOnly,
    /// Process all messages in the channel.
    AllMessages,
}

/// Result of an access control check.
#[derive(Debug)]
pub enum AccessDecision {
    /// Message is allowed through.
    Allowed,
    /// Message is blocked (silently dropped).
    Blocked,
    /// A pairing code was sent to the user; message is not processed.
    PairingSent,
}
