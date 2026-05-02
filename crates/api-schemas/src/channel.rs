//! Channel API request/response schemas.

use serde::{Deserialize, Serialize};

use crate::enums::{ChannelType, RoutingMode, Scope};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateChannelConfigRequest {
    pub provider: ChannelType,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub provider_config: Option<serde_json::Value>,
    pub response_config_id: Option<String>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Option<Vec<String>>,
    pub run_as_user_id: Option<String>,
    pub allow_direct_send: Option<bool>,
    /// Agent type for tasks created from this channel (defaults to "claude-code")
    pub agent_type: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannelConfigRequest {
    pub source_name: Option<String>,
    pub repository_id: Option<String>,
    pub routing_mode: Option<RoutingMode>,
    pub prompt_template: Option<Option<String>>,
    pub active: Option<bool>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub provider_config: Option<serde_json::Value>,
    pub response_config_id: Option<Option<String>>,
    pub task_name: Option<Option<String>>,
    pub tag_ids: Option<Vec<String>>,
    pub run_as_user_id: Option<Option<String>>,
    pub allow_direct_send: Option<bool>,
    /// Agent type for tasks created from this channel
    pub agent_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfigResponse {
    pub id: String,
    pub provider: ChannelType,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    pub active: bool,
    pub allow_direct_send: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub response_config_id: Option<String>,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_by: String,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    /// Agent type for tasks created from this channel
    pub agent_type: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Request to send a message through channel(s) associated with a task.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendChannelMessageRequest {
    pub text: String,
    /// File IDs (from `/api/v1/uploads`) to attach to the message.
    #[serde(default)]
    pub file_ids: Vec<String>,
}

/// Response from sending a message through channel(s).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendChannelMessageResponse {
    pub sent_count: u32,
    pub errors: Vec<String>,
}

/// One outbound destination listed in a [`MultipleTargetsErrorBody`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ChannelTargetDescriptor {
    /// `"thread"` for legacy `channel_threads` rows, `"subscription"` for
    /// `task_subscriptions` rows.
    pub kind: String,
    /// Stable identifier — `thread_id` (stringified i32) or `subscription_id`.
    pub id: String,
    /// Human-readable label such as `"slack #ops"` or `"github_pr owner/repo#42"`.
    pub label: String,
    /// CLI flag the caller should add to disambiguate, e.g.
    /// `"--thread 42"` or `"--subscription sub_xyz"`.
    pub flag_hint: String,
}

/// Body returned with HTTP 400 when `POST /channels/send` cannot pick a
/// single destination because the task is wired to multiple channels.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct MultipleTargetsErrorBody {
    pub error: String,
    pub targets: Vec<ChannelTargetDescriptor>,
    pub hint: String,
}

/// Request to send a message directly to a channel config.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct DirectSendRequest {
    pub text: String,
    /// File IDs (from `/api/v1/uploads`) to attach to the message.
    #[serde(default)]
    pub file_ids: Vec<String>,
    /// Optional task ID. When set, the first send creates a thread linked to
    /// this task; subsequent sends reply in that thread.
    pub task_id: Option<String>,
}

/// Response from sending a message directly to a channel config.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct DirectSendResponse {
    /// Whether the message was sent successfully.
    pub success: bool,
    /// Provider-specific external message ID (e.g. Slack message ts), if available.
    pub external_message_id: Option<String>,
}

/// Channel connected to a task (via channel_threads or task_subscriptions).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskChannelResponse {
    pub config_id: String,
    pub provider: ChannelType,
    pub source_id: String,
    pub source_name: Option<String>,
    /// Internal channel thread identifier. Use this for exact outbound targeting
    /// when a task has multiple subscriptions under the same config.
    pub thread_id: Option<i32>,
    /// Provider thread identifier such as GitHub PR number or Slack thread ts.
    pub external_thread_id: Option<String>,
    /// Provider-specific subject key such as `owner/repo#123` for GitHub PR subscriptions.
    pub subject_key: Option<String>,
    /// `task_subscriptions.id` (string surrogate) when this row originated from
    /// the GitHub subscription model. None for legacy thread-based providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_id: Option<String>,
    /// `task_subscriptions.created_reason` (e.g. `ingress`, `connected_thread`).
    /// None for legacy thread-based providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_reason: Option<String>,
    /// ISO-8601 timestamp of the last event seen on the subscription. None for
    /// legacy thread-based providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SetChannelDefaultsRequest {
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub provider_config: Option<serde_json::Value>,
    pub response_config_id: Option<String>,
    pub team_id: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Option<Vec<String>>,
    /// Agent type for tasks created from this channel (defaults to "claude-code")
    pub agent_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDefaultsQuery {
    pub team_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListChannelConfigsQuery {
    pub provider: Option<String>,
    pub repository_id: Option<String>,
}

/// Available Slack channel info for dropdown selection.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SlackChannelInfoResponse {
    pub id: String,
    pub name: String,
}

/// Request body for manually triggering a channel config.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TriggerChannelRequest {
    /// Optional message override. If omitted, uses the channel's configured message.
    pub message: Option<String>,
    /// Optional thread properties for the created task. Required for `github_pr`
    /// channels (must include `owner`, `repo`, and `pr_number`).
    pub thread_properties: Option<serde_json::Value>,
}

/// Response from manually triggering a channel config.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TriggerChannelResponse {
    pub task_id: String,
    pub short_id: String,
}

/// Request to connect a task to an existing external thread (e.g. a GitHub PR).
///
/// Creates a channel thread linking the task to the external thread so that
/// inbound messages (webhooks) are delivered and the agent can reply.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ConnectChannelRequest {
    pub task_id: String,
    pub provider: ChannelType,
    /// Source identifier, e.g. `"owner/repo"` for GitHub PR.
    /// Resolved from the task's repository if omitted.
    pub source_id: Option<String>,
    /// External thread identifier, e.g. PR number `"42"`.
    pub external_thread_id: String,
    /// Provider-specific properties (e.g. `{"owner","repo","pr_number"}`).
    /// Auto-generated from source_id + external_thread_id for github_pr if omitted.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub thread_properties: Option<serde_json::Value>,
    /// System user ID whose messages should be filtered out on this thread
    /// (prevents echoing the agent's own PR comments back to it).
    pub filter_user_id: Option<String>,
}

/// Response from connecting a task to a channel thread.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ConnectChannelResponse {
    pub config_id: String,
    pub thread_id: i32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingQuery {
    pub config_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ApprovePairingRequest {
    pub config_id: String,
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PairingRequestResponse {
    pub id: i32,
    pub config_id: String,
    pub provider_user_id: String,
    pub code: String,
    pub status: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub meta: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Provider connections + ingress rules (GitHub-native flow)
// ---------------------------------------------------------------------------

/// One row from `provider_connections` — describes a transport/auth scope
/// (e.g. one GitHub org webhook).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionResponse {
    pub id: String,
    pub provider: String,
    pub connection_kind: String,
    pub scope_value: String,
    pub created_by: String,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Request body for `POST /api/v1/channels/rules`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateIngressRuleRequest {
    /// Provider value, e.g. `"github"`, `"slack"`, or `"telegram"`.
    pub provider: String,
    /// Provider source matcher: repo full name (`owner/repo`) for GitHub,
    /// Slack channel id, Telegram chat id, or `*` wildcard.
    pub scope_matcher: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: Option<RoutingMode>,
    pub task_name: Option<String>,
    pub tag_ids: Option<Vec<String>>,
    pub prompt_template: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub provider_config: Option<serde_json::Value>,
    pub allow_direct_send: Option<bool>,
    pub response_rule_id: Option<String>,
    pub ongoing_task_id: Option<String>,
    /// Defaults to `"claude-code"` when omitted.
    pub agent_type: Option<String>,
    pub run_as_user_id: Option<String>,
    pub close_task_on_close: Option<bool>,
    pub notify_on_success: Option<bool>,
    pub active: Option<bool>,
    /// Optional event filter — empty means all supported events.
    pub event_filter: Option<Vec<String>>,
    /// Required when the caller belongs to multiple teams. Defaults to the
    /// caller's only team when single-team.
    pub team_id: Option<String>,
}

/// One row from `ingress_rules`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct IngressRuleResponse {
    pub id: String,
    pub provider: String,
    pub connection_id: String,
    pub scope_matcher: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub prompt_template: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub provider_config: Option<serde_json::Value>,
    pub allow_direct_send: bool,
    pub response_rule_id: Option<String>,
    pub ongoing_task_id: Option<String>,
    pub agent_type: String,
    pub run_as_user_id: Option<String>,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
    pub active: bool,
    pub event_filter: Vec<String>,
    pub created_by: String,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
