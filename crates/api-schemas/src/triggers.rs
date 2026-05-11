//! Unified trigger API schemas.

use serde::{Deserialize, Serialize};

use crate::bindings::TaskBindingResponse;
use crate::enums::{AdapterKind, RoutingMode, Scope, TaskStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    AdapterEvent,
    Schedule,
    Manual,
}

impl TriggerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AdapterEvent => "adapter_event",
            Self::Schedule => "schedule",
            Self::Manual => "manual",
        }
    }
}

impl TryFrom<&str> for TriggerKind {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "adapter_event" => Ok(Self::AdapterEvent),
            "schedule" => Ok(Self::Schedule),
            "manual" => Ok(Self::Manual),
            other => Err(format!("unknown TriggerKind: {other:?}")),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TriggerRule {
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub task_name: Option<String>,
    pub prompt: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateTriggerRule {
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub task_name: Option<String>,
    pub prompt: Option<String>,
    pub tag_ids: Option<Vec<String>>,
    pub run_as_user_id: Option<String>,
    pub agent_type: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateTriggerRule {
    pub repository_id: Option<String>,
    pub routing_mode: Option<RoutingMode>,
    pub ongoing_task_id: Option<Option<String>>,
    pub task_name: Option<Option<String>>,
    pub prompt: Option<Option<String>>,
    pub tag_ids: Option<Vec<String>>,
    pub run_as_user_id: Option<Option<String>>,
    pub agent_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdapterEventTriggerSource {
    pub adapter_kind: AdapterKind,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub event_filter: serde_json::Value,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub adapter_config: Option<serde_json::Value>,
    pub allow_direct_send: bool,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateAdapterEventTriggerSource {
    pub adapter_kind: AdapterKind,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub event_filter: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub adapter_config: Option<serde_json::Value>,
    pub allow_direct_send: Option<bool>,
    pub close_task_on_close: Option<bool>,
    pub notify_on_success: Option<bool>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UpdateAdapterEventTriggerSource {
    pub connection_id: Option<String>,
    pub source_matcher: Option<String>,
    pub source_name: Option<Option<String>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub event_filter: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub adapter_config: Option<Option<serde_json::Value>>,
    pub allow_direct_send: Option<bool>,
    pub close_task_on_close: Option<bool>,
    pub notify_on_success: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ScheduleTriggerSource {
    pub cron_expression: String,
    pub timezone: String,
    pub message_template: String,
    pub last_fired_at: Option<String>,
    pub next_fire_at: String,
    pub locked_by: Option<String>,
    pub locked_until: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateScheduleTriggerSource {
    pub cron_expression: String,
    pub timezone: Option<String>,
    pub message_template: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UpdateScheduleTriggerSource {
    pub cron_expression: Option<String>,
    pub timezone: Option<String>,
    pub message_template: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ManualTriggerSource {}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum TriggerSource {
    AdapterEvent(AdapterEventTriggerSource),
    Schedule(ScheduleTriggerSource),
    Manual(ManualTriggerSource),
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum CreateTriggerSource {
    AdapterEvent(CreateAdapterEventTriggerSource),
    Schedule(CreateScheduleTriggerSource),
    Manual(ManualTriggerSource),
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum UpdateTriggerSource {
    AdapterEvent(UpdateAdapterEventTriggerSource),
    Schedule(UpdateScheduleTriggerSource),
    Manual(ManualTriggerSource),
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TriggerResponse {
    pub id: String,
    pub name: Option<String>,
    pub kind: TriggerKind,
    pub active: bool,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub rule: TriggerRule,
    pub source: TriggerSource,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateTriggerRequest {
    pub name: Option<String>,
    pub kind: TriggerKind,
    pub active: Option<bool>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
    pub rule: CreateTriggerRule,
    pub source: CreateTriggerSource,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateTriggerRequest {
    pub name: Option<Option<String>>,
    pub active: Option<bool>,
    pub scope: Option<Scope>,
    pub team_id: Option<Option<String>>,
    pub rule: Option<UpdateTriggerRule>,
    pub source: Option<UpdateTriggerSource>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TriggerRecentTaskSummary {
    pub task_id: String,
    pub short_id: Option<String>,
    pub title: String,
    pub status: TaskStatus,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TriggerRecentTaskItem {
    pub task: TriggerRecentTaskSummary,
    pub binding: TaskBindingResponse,
}
