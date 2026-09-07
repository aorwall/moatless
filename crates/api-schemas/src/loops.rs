//! Loop API schemas.
//!
//! A Loop is a standing definition of recurring agent work (CONTEXT.md,
//! the Loop model decision) that fires on a Schedule or a Subscription and drives one or more
//! Tasks. These types are the `/api/v1/loops` request/response DTOs plus the
//! repo-sync outcome types for `.moatless/loops/<name>/LOOP.md` reconciliation.

use serde::{Deserialize, Serialize};

use crate::bindings::TaskBindingResponse;
use crate::enums::{AdapterKind, RoutingMode, Scope, TaskStatus};
use crate::patch::deserialize_optional_nullable_field;
use crate::repository::RepositoryResponse;
use crate::workspace::WorkspaceSyncSummary;

pub use moatless_types::loops::LoopKind;

pub use moatless_types::loops::LoopConfigSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LoopExecutionState {
    AwaitingApproval,
    Active,
    Paused,
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
    /// Skills every task this Loop spawns activates on each fresh agent session.
    pub skills: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    /// Model the spawned Tasks run on, from the `agent_type` entry's catalog
    /// models. `None` means "inherit the platform default the sandbox resolves
    /// for `agent_type`". Flows into `tasks.model` when this Loop spawns a task.
    pub model: Option<String>,
    /// Per-Loop inactivity TTL in days. `None` means "inherit the global
    /// default" (`TASK_INACTIVE_TIMEOUT_DAYS`). Flows into `tasks.inactive_ttl_days`
    /// when this Loop spawns a task.
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
    pub skills: Option<Vec<String>>,
    pub run_as_user_id: Option<String>,
    pub agent_type: Option<String>,
    /// See [`LoopConfig::model`].
    pub model: Option<String>,
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
    pub skills: Option<Vec<String>>,
    pub agent_type: Option<String>,
    /// See [`LoopConfig::model`]. `Some(None)` clears the override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_nullable_field"
    )]
    pub model: Option<Option<String>>,
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
    /// Human-readable name. Required on every Loop — for Git-declared Loops it
    /// comes from the `name` frontmatter of `.moatless/loops/<dir>/LOOP.md`.
    pub name: String,
    pub kind: LoopKind,
    pub active: bool,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub config_source: LoopConfigSource,
    pub synced_from_git: bool,
    pub source_repository_id: Option<String>,
    pub source_key: Option<String>,
    pub source_config_path: Option<String>,
    pub deleted: bool,
    pub execution_state: LoopExecutionState,
    pub config: LoopConfig,
    pub source: LoopSource,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateLoopRequest {
    /// Human-readable name. Required and must not be blank.
    pub name: String,
    pub kind: LoopKind,
    pub active: Option<bool>,
    pub scope: Option<Scope>,
    pub config: CreateLoopConfig,
    pub source: CreateLoopSource,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateLoopRequest {
    /// Rename the Loop. Omit to keep the current name; a name cannot be cleared.
    pub name: Option<String>,
    pub scope: Option<Scope>,
    pub config: Option<UpdateLoopConfig>,
    pub source: Option<UpdateLoopSource>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ActivateLoopRequest {
    pub run_as_user_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ResetGitLoopResponse {
    #[serde(rename = "loop")]
    pub loop_: Loop,
    pub resynced: bool,
    pub error: Option<String>,
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
    /// `None` for schedule loops (no return-channel binding) and for
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LoopSyncWarning {
    pub loop_name: String,
    pub field: String,
    pub message: String,
}

/// Summary of reconciling a repo's `.moatless/loops/` into Loop tables.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LoopSyncSummary {
    /// Loops inserted.
    pub created: u32,
    /// Existing Loops updated in place.
    pub updated: u32,
    /// Loops removed because their directory is gone.
    pub removed: u32,
    /// Manual overrides or deletion tombstones preserved during repository sync.
    pub overrides_skipped: u32,
    /// Loops that were present but not synced, with reasons.
    pub skipped: Vec<LoopSyncSkip>,
    /// Non-fatal authoring warnings, including ignored Git authority fields.
    pub warnings: Vec<LoopSyncWarning>,
}

/// Response for `POST /api/v1/repositories/{id}/sync-config`.
///
/// Wraps the updated repository plus the loop and workspace reconciliation
/// summaries. This endpoint is the explicit, user-initiated sync, so it also
/// reclaims workspaces that were manually overridden — see
/// [`WorkspaceSyncSummary::reclaimed`].
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SyncConfigResponse {
    pub repository: RepositoryResponse,
    pub loops: LoopSyncSummary,
    pub workspaces: WorkspaceSyncSummary,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::UpdateLoopConfig;

    #[test]
    fn update_loop_config_distinguishes_omitted_null_and_value() {
        let omitted: UpdateLoopConfig = serde_json::from_value(json!({})).unwrap();
        assert_eq!(omitted.model, None);

        let cleared: UpdateLoopConfig = serde_json::from_value(json!({ "model": null })).unwrap();
        assert_eq!(cleared.model, Some(None));

        let set: UpdateLoopConfig = serde_json::from_value(json!({ "model": "fable" })).unwrap();
        assert_eq!(set.model, Some(Some("fable".to_string())));
    }

    /// The CLI sends this struct, so an omitted `model` must not reach the
    /// server as a clear.
    #[test]
    fn an_omitted_model_survives_a_round_trip_as_omitted() {
        let round_trip = |patch: UpdateLoopConfig| -> UpdateLoopConfig {
            serde_json::from_str(&serde_json::to_string(&patch).unwrap()).unwrap()
        };

        assert_eq!(round_trip(UpdateLoopConfig::default()).model, None);
        assert_eq!(
            round_trip(UpdateLoopConfig {
                model: Some(None),
                ..UpdateLoopConfig::default()
            })
            .model,
            Some(None)
        );
        assert_eq!(
            round_trip(UpdateLoopConfig {
                model: Some(Some("fable".into())),
                ..UpdateLoopConfig::default()
            })
            .model,
            Some(Some("fable".to_string()))
        );
    }
}
