//! Channel API request/response schemas.

use serde::{Deserialize, Serialize};

use crate::enums::{IntegrationProvider, RoutingMode, Scope};

/// One outbound destination returned by task reply APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ChannelTargetDescriptor {
    /// Target category, currently `"binding"` for canonical task bindings.
    pub kind: String,
    /// Stable target identifier, usually `task_bindings.id`.
    pub id: String,
    /// Human-readable label such as `"slack channel:C123"` or `"github pr:owner/repo#42"`.
    pub label: String,
    /// CLI flag the caller should add to disambiguate, e.g. `"--binding bind_xyz"`.
    pub flag_hint: String,
}

/// Body returned with HTTP 400 when `POST /tasks/{task_id}/reply` cannot pick a
/// single destination because the task is wired to multiple bindings.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct MultipleTargetsErrorBody {
    pub error: String,
    pub targets: Vec<ChannelTargetDescriptor>,
    pub hint: String,
}

/// External integration binding connected to a task.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskChannelResponse {
    pub trigger_id: String,
    pub provider: IntegrationProvider,
    pub source_id: String,
    pub source_name: Option<String>,
    /// Deprecated legacy thread identifier. Canonical callers should use
    /// `binding_id`, which carries the task binding id.
    pub thread_id: Option<i32>,
    /// Provider thread identifier such as GitHub PR number or Slack thread ts.
    pub external_thread_id: Option<String>,
    /// Provider-specific subject key such as `owner/repo#123` for GitHub PR bindings.
    pub subject_key: Option<String>,
    /// Canonical `task_bindings.id` when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    /// Binding origin (e.g. `ingress`, `direct_send`, `scheduled_job`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// ISO-8601 timestamp of the last event seen on the binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Provider connections
// ---------------------------------------------------------------------------

/// Request body for `POST /api/v1/integrations/connections`.
///
/// Today only used for Linear, where the workspace connection has to be
/// registered explicitly because each Linear workspace creates its own
/// webhook with its own secret.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateProviderConnectionRequest {
    /// Provider value.
    pub provider: IntegrationProvider,
    /// Transport/auth style for this provider, e.g. `bot_api`, `org_webhook`.
    pub connection_kind: Option<String>,
    /// External account/workspace/org identifier for this connection.
    pub external_account_id: String,
    /// HMAC secret used to verify inbound webhooks for this workspace.
    pub webhook_secret: Option<String>,
    /// Optional human-readable name for the connection.
    pub source_name: Option<String>,
    /// Optional transport-specific metadata (merged into `transport_config`).
    /// Linear example: `{"workspace_url_key":"acme"}`.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub transport_config: Option<serde_json::Value>,
    /// Required when the caller belongs to multiple teams.
    pub team_id: Option<String>,
}

/// One row from `provider_connections` — describes a transport/auth scope
/// (e.g. one GitHub org webhook).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionResponse {
    pub id: String,
    pub provider: IntegrationProvider,
    pub connection_kind: String,
    pub external_account_id: String,
    pub created_by: String,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Canonical integration model
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskRouteResponse {
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
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskRouteRequest {
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Option<Vec<String>>,
    pub run_as_user_id: Option<String>,
    pub agent_type: Option<String>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskRouteRequest {
    pub repository_id: Option<String>,
    pub routing_mode: Option<RoutingMode>,
    pub ongoing_task_id: Option<Option<String>>,
    pub prompt_template: Option<Option<String>>,
    pub task_name: Option<Option<String>>,
    pub tag_ids: Option<Vec<String>>,
    pub run_as_user_id: Option<Option<String>>,
    pub agent_type: Option<String>,
    pub scope: Option<Scope>,
    pub team_id: Option<Option<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskTriggerResponse {
    pub id: String,
    pub provider: IntegrationProvider,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub event_filter: serde_json::Value,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub trigger_config: Option<serde_json::Value>,
    pub route_id: String,
    pub allow_direct_send: bool,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
    pub active: bool,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskTriggerRequest {
    pub provider: IntegrationProvider,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub event_filter: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub trigger_config: Option<serde_json::Value>,
    pub route_id: String,
    pub allow_direct_send: Option<bool>,
    pub close_task_on_close: Option<bool>,
    pub notify_on_success: Option<bool>,
    pub active: Option<bool>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskTriggerRequest {
    pub connection_id: Option<String>,
    pub source_matcher: Option<String>,
    pub source_name: Option<Option<String>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub event_filter: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub trigger_config: Option<Option<serde_json::Value>>,
    pub route_id: Option<String>,
    pub allow_direct_send: Option<bool>,
    pub close_task_on_close: Option<bool>,
    pub notify_on_success: Option<bool>,
    pub active: Option<bool>,
    pub scope: Option<Scope>,
    pub team_id: Option<Option<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskBindingResponse {
    pub id: String,
    pub task_id: String,
    pub provider: IntegrationProvider,
    pub connection_id: String,
    pub trigger_id: Option<String>,
    pub subject_kind: String,
    pub subject_key: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub destination_config: serde_json::Value,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    pub origin: String,
    pub created_at: String,
    pub last_event_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskBindingRequest {
    pub provider: IntegrationProvider,
    pub connection_id: String,
    pub trigger_id: Option<String>,
    pub subject_kind: String,
    pub subject_key: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub destination_config: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub metadata: Option<serde_json::Value>,
    pub origin: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ScheduledJobResponse {
    pub id: String,
    pub route_id: String,
    pub schedule_expr: String,
    pub timezone: String,
    pub message_template: String,
    pub run_as_user_id: Option<String>,
    pub active: bool,
    pub last_fired_at: Option<String>,
    pub next_fire_at: String,
    pub locked_by: Option<String>,
    pub locked_until: Option<String>,
    pub created_by: String,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateScheduledJobRequest {
    pub route_id: String,
    pub schedule_expr: String,
    pub timezone: Option<String>,
    pub message_template: String,
    pub run_as_user_id: Option<String>,
    pub active: Option<bool>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateScheduledJobRequest {
    pub route_id: Option<String>,
    pub schedule_expr: Option<String>,
    pub timezone: Option<String>,
    pub message_template: Option<String>,
    pub run_as_user_id: Option<Option<String>>,
    pub active: Option<bool>,
    pub scope: Option<Scope>,
    pub team_id: Option<Option<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TriggerRecentTaskSummary {
    pub task_id: String,
    pub short_id: Option<String>,
    pub title: String,
    pub status: crate::enums::TaskStatus,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TriggerRecentTaskItem {
    pub task: TriggerRecentTaskSummary,
    pub binding: TaskBindingResponse,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendTaskReplyRequest {
    pub text: String,
    #[serde(default)]
    pub file_ids: Vec<String>,
    pub binding_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendTaskReplyResponse {
    pub sent_count: u32,
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<ChannelTargetDescriptor>,
}
