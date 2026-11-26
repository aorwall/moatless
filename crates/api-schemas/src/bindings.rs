//! Task binding and outbound reply API schemas.

use serde::{Deserialize, Serialize};

use crate::enums::AdapterKind;

/// One outbound destination returned by task reply APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReplyTargetDescriptor {
    /// Target category, currently `"binding"` for canonical task bindings.
    pub kind: String,
    /// Stable target identifier, usually `task_bindings.id`.
    pub id: String,
    /// Human-readable label such as `"slack source:C123"` or `"github
    /// pr:owner/repo#42"`.
    pub label: String,
    /// CLI flag the caller should add to disambiguate, e.g. `"--binding
    /// bind_xyz"`.
    pub flag_hint: String,
}

/// Body returned with HTTP 400 when `POST /tasks/{task_id}/reply` cannot pick a
/// single destination because the task is wired to multiple bindings.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct MultipleTargetsErrorBody {
    pub error: String,
    pub targets: Vec<ReplyTargetDescriptor>,
    pub hint: String,
}

/// External adapter binding connected to a task.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskBindingResponse {
    pub id: String,
    pub task_id: String,
    pub adapter_kind: AdapterKind,
    pub connection_id: String,
    pub loop_id: Option<String>,
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
    pub adapter_kind: AdapterKind,
    pub connection_id: String,
    pub loop_id: Option<String>,
    pub subject_kind: String,
    pub subject_key: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub destination_config: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub metadata: Option<serde_json::Value>,
    pub origin: Option<String>,
    /// Close the task when the subject closes. Rejected for any adapter but
    /// `github`, whose pull requests are the only subjects that close.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub close_task_on_close: bool,
}

/// External adapter binding summary attached to a task response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskBindingSummary {
    pub loop_id: String,
    pub adapter_kind: AdapterKind,
    pub source_id: String,
    pub source_name: Option<String>,
    /// Adapter thread identifier such as GitHub PR number or Slack thread ts.
    pub external_thread_id: Option<String>,
    /// Adapter-specific subject key such as `owner/repo#123` for GitHub PR
    /// bindings.
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

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendTaskReplyRequest {
    pub text: String,
    #[serde(default)]
    pub file_ids: Vec<String>,
    pub binding_id: Option<String>,
    /// Adapter-native rich rendering of the message body. The payload names the
    /// adapter it addresses; one whose kind does not match the resolved
    /// destination is rejected rather than dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<moatless_types::adapter::OutboundPresentation>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendTaskReplyResponse {
    pub sent_count: u32,
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<ReplyTargetDescriptor>,
}
