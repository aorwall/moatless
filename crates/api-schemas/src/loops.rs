//! Loop API schemas.
//!
//! A Loop is a standing definition of recurring agent work (CONTEXT.md,
//! ADR-0001) that fires on a Schedule or a Subscription and drives one or more
//! Tasks. These types are the `/api/v1/loops` request/response DTOs plus the
//! repo-sync outcome types for `.moatless/loops/<name>/LOOP.md` reconciliation.

use serde::{Deserialize, Serialize};

use crate::bindings::TaskBindingResponse;
use crate::enums::{AdapterKind, RoutingMode, Scope, TaskStatus};
use crate::repository::RepositoryResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LoopKind {
    AdapterEvent,
    Schedule,
    Manual,
}

impl LoopKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AdapterEvent => "adapter_event",
            Self::Schedule => "schedule",
            Self::Manual => "manual",
        }
    }
}

impl TryFrom<&str> for LoopKind {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "adapter_event" => Ok(Self::AdapterEvent),
            "schedule" => Ok(Self::Schedule),
            "manual" => Ok(Self::Manual),
            other => Err(format!("unknown LoopKind: {other:?}")),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LoopConfig {
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub task_name: Option<String>,
    pub prompt: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    /// Per-trigger inactivity TTL in days. `None` means "inherit the global
    /// default" (`TASK_INACTIVE_TIMEOUT_DAYS`). Flows into `tasks.inactive_ttl_days`
    /// when this trigger spawns a task.
    pub inactive_ttl_days: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateLoopConfig {
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub task_name: Option<String>,
    pub prompt: Option<String>,
    pub tag_ids: Option<Vec<String>>,
    pub run_as_user_id: Option<String>,
    pub agent_type: Option<String>,
    pub inactive_ttl_days: Option<i32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateLoopConfig {
    pub repository_id: Option<String>,
    pub routing_mode: Option<RoutingMode>,
    pub ongoing_task_id: Option<Option<String>>,
    pub task_name: Option<Option<String>>,
    pub prompt: Option<Option<String>>,
    pub tag_ids: Option<Vec<String>>,
    pub run_as_user_id: Option<Option<String>>,
    pub agent_type: Option<String>,
    pub inactive_ttl_days: Option<Option<i32>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
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
pub struct CreateSubscription {
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
pub struct UpdateSubscription {
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
pub struct Schedule {
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
pub struct CreateSchedule {
    pub cron_expression: String,
    pub timezone: Option<String>,
    pub message_template: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UpdateSchedule {
    pub cron_expression: Option<String>,
    pub timezone: Option<String>,
    pub message_template: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ManualSource {}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum LoopSource {
    AdapterEvent(Subscription),
    Schedule(Schedule),
    Manual(ManualSource),
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum CreateLoopSource {
    AdapterEvent(CreateSubscription),
    Schedule(CreateSchedule),
    Manual(ManualSource),
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum UpdateLoopSource {
    AdapterEvent(UpdateSubscription),
    Schedule(UpdateSchedule),
    Manual(ManualSource),
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Loop {
    pub id: String,
    pub name: Option<String>,
    pub kind: LoopKind,
    pub active: bool,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(rename = "rule")]
    pub config: LoopConfig,
    pub source: LoopSource,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateLoopRequest {
    pub name: Option<String>,
    pub kind: LoopKind,
    pub active: Option<bool>,
    pub scope: Option<Scope>,
    #[serde(rename = "rule")]
    pub config: CreateLoopConfig,
    pub source: CreateLoopSource,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateLoopRequest {
    pub name: Option<Option<String>>,
    pub active: Option<bool>,
    pub scope: Option<Scope>,
    #[serde(rename = "rule")]
    pub config: Option<UpdateLoopConfig>,
    pub source: Option<UpdateLoopSource>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LoopRecentTaskSummary {
    pub task_id: String,
    pub short_id: Option<String>,
    pub title: String,
    pub status: TaskStatus,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LoopRecentTaskItem {
    pub task: LoopRecentTaskSummary,
    /// `None` for schedule triggers (no return-channel binding) and for
    /// adapter-event tasks that lost their original binding.
    pub binding: Option<TaskBindingResponse>,
}

// ---- Repo loop sync (`.moatless/loops/<name>/LOOP.md`) ----

/// One loop that was skipped during a sync, with a human-readable reason
/// (e.g. "no github connection for org acme").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LoopSyncSkip {
    /// The loop directory name (`.moatless/loops/<loop>/`).
    pub loop_name: String,
    /// Why the loop was skipped.
    pub reason: String,
}

/// Summary of reconciling a repo's `.moatless/loops/` into the trigger tables.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LoopSyncSummary {
    /// Loops inserted as new triggers.
    pub created: u32,
    /// Existing loop triggers updated in place.
    pub updated: u32,
    /// Loop triggers removed because their directory is gone.
    pub removed: u32,
    /// Loops that were present but not synced, with reasons.
    pub skipped: Vec<LoopSyncSkip>,
}

/// Response for `POST /api/v1/repositories/{id}/sync-config`.
///
/// Wraps the updated repository plus the loop reconciliation summary.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SyncConfigResponse {
    pub repository: RepositoryResponse,
    pub loops: LoopSyncSummary,
}
