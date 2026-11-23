//! Loop API schemas.
//!
//! A Loop is a standing definition of recurring agent work (CONTEXT.md, the
//! Loop model decision) that fires on a Schedule or a Subscription and drives
//! one or more Tasks. These types are the `/api/v1/loops` request/response DTOs
//! plus the repo-sync outcome types for `.moatless/loops/<name>/LOOP.md`
//! reconciliation.

use serde::{Deserialize, Serialize};

use crate::bindings::TaskBindingResponse;
use crate::enums::{AdapterKind, RoutingMode, Scope, TaskStatus};
use crate::patch::deserialize_optional_nullable_field;
use moatless_types::agent::AgentProperties;
use moatless_workspace_types::schemas::repository::RepositoryResponse;
use moatless_workspace_types::schemas::workspace::WorkspaceSyncSummary;

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
    /// The Workspace every task this Loop spawns runs in. Its primary placement
    /// is the repository those tasks run against.
    pub workspace_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub task_name: Option<String>,
    pub prompt: Option<String>,
    pub tag_ids: Vec<String>,
    /// Memory keys, or key prefixes ending in `*`, read from this Loop's scope
    /// and listed in each spawned Task's prompt. A `LOOP.md` sets them with
    /// `memory.preload`.
    #[serde(default)]
    pub memory_preload: Vec<String>,
    /// Skills every task this Loop spawns activates on each fresh agent
    /// session.
    pub skills: Vec<String>,
    /// Harness settings every task this Loop spawns runs with. A `LOOP.md`
    /// sets them with `agentProperties`.
    #[serde(default)]
    pub agent_properties: AgentProperties,
    pub run_as_user_id: Option<String>,
    pub agent_profile: String,
    /// Model the spawned Tasks run on, from the `agent_profile` entry's catalog
    /// models. `None` means "inherit the platform default the sandbox resolves
    /// for `agent_profile`". Flows into `tasks.model` when this Loop spawns a
    /// task.
    pub model: Option<String>,
    /// See [`moatless_types::loops::LoopContext::close_after_settled_days`].
    #[serde(alias = "inactiveTtlDays")]
    pub close_after_settled_days: Option<i32>,
    /// See [`moatless_types::loops::LoopContext::settle_after_hours`]; `0`
    /// never auto-settles.
    pub settle_after_hours: Option<i32>,
    /// Extra node labels for the sandboxes this Loop spawns, merged over the
    /// deployment-wide selector and outranking the Workspace's own value.
    /// Admin-only, and never declared in `LOOP.md`.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub node_selector: Option<std::collections::BTreeMap<String, String>>,
    /// Stop this Loop's sandbox as soon as its agent run finishes, rather than
    /// holding the pod for the idle TTL.
    pub stop_when_done: Option<bool>,
    /// Longest, in seconds, inbound Adapter events for one Binding are held so
    /// those arriving together reach the Task as a single message; a lone
    /// event leaves after a short quiet period. `None` inherits
    /// the deployment default for the Loop's adapter kind (60 for `github_pr`,
    /// off elsewhere); `0` delivers every event on its own.
    pub message_buffer_seconds: Option<i32>,
    /// Re-run a spawned Task's failed turn once on another agent, in the same
    /// Task, when its provider will not serve it; see
    /// `moatless_server::event_handler::agent_fallback`. A Task no Loop
    /// created always falls back.
    pub fallback_to_other_agent: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateLoopConfig {
    /// See [`LoopConfig::workspace_id`].
    pub workspace_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub task_name: Option<String>,
    pub prompt: Option<String>,
    pub tag_ids: Option<Vec<String>>,
    /// See [`LoopConfig::memory_preload`].
    #[serde(default)]
    pub memory_preload: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    /// See [`LoopConfig::agent_properties`].
    #[serde(default)]
    pub agent_properties: Option<AgentProperties>,
    pub run_as_user_id: Option<String>,
    pub agent_profile: Option<String>,
    /// See [`LoopConfig::model`].
    pub model: Option<String>,
    #[serde(alias = "inactiveTtlDays")]
    pub close_after_settled_days: Option<i32>,
    /// See [`LoopConfig::settle_after_hours`].
    pub settle_after_hours: Option<i32>,
    /// See [`LoopConfig::node_selector`].
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub node_selector: Option<std::collections::BTreeMap<String, String>>,
    /// See [`LoopConfig::stop_when_done`].
    pub stop_when_done: Option<bool>,
    /// See [`LoopConfig::message_buffer_seconds`].
    pub message_buffer_seconds: Option<i32>,
    /// See [`LoopConfig::fallback_to_other_agent`]. `None` is on.
    pub fallback_to_other_agent: Option<bool>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateLoopConfig {
    /// See [`LoopConfig::workspace_id`].
    pub workspace_id: Option<String>,
    pub routing_mode: Option<RoutingMode>,
    /// `Some(None)` clears the override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_nullable_field"
    )]
    pub ongoing_task_id: Option<Option<String>>,
    /// `Some(None)` clears the override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_nullable_field"
    )]
    pub task_name: Option<Option<String>>,
    /// `Some(None)` clears the override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_nullable_field"
    )]
    pub prompt: Option<Option<String>>,
    pub tag_ids: Option<Vec<String>>,
    /// See [`LoopConfig::memory_preload`]. `Some(vec![])` clears it.
    #[serde(default)]
    pub memory_preload: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    /// See [`LoopConfig::agent_properties`]. Replaces the whole set, so `{}`
    /// clears it.
    #[serde(default)]
    pub agent_properties: Option<AgentProperties>,
    pub agent_profile: Option<String>,
    /// See [`LoopConfig::model`]. `Some(None)` clears the override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_nullable_field"
    )]
    pub model: Option<Option<String>>,
    /// See [`LoopConfig::close_after_settled_days`]. `Some(None)` clears the override.
    #[serde(
        default,
        alias = "inactiveTtlDays",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_nullable_field"
    )]
    pub close_after_settled_days: Option<Option<i32>>,
    /// See [`LoopConfig::settle_after_hours`]. `Some(None)` clears the override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_nullable_field"
    )]
    pub settle_after_hours: Option<Option<i32>>,
    /// See [`LoopConfig::node_selector`]. `Some(None)` clears the override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_nullable_field"
    )]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub node_selector: Option<Option<std::collections::BTreeMap<String, String>>>,
    /// See [`LoopConfig::stop_when_done`].
    pub stop_when_done: Option<bool>,
    /// See [`LoopConfig::message_buffer_seconds`]. `Some(None)` clears the
    /// override.
    pub message_buffer_seconds: Option<Option<i32>>,
    /// See [`LoopConfig::fallback_to_other_agent`]. `None` keeps the current
    /// value.
    pub fallback_to_other_agent: Option<bool>,
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
    /// `Some(None)` clears the override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_nullable_field"
    )]
    pub source_name: Option<Option<String>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub event_filter: Option<serde_json::Value>,
    /// `Some(None)` clears the override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_nullable_field"
    )]
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
    /// Rename the Loop. Omit to keep the current name; a name cannot be
    /// cleared.
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

/// One loop that was skipped during a sync, with a human-readable reason (e.g.
/// "no github connection for org acme").
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
    /// Manual overrides or deletion tombstones preserved during repository
    /// sync.
    pub overrides_skipped: u32,
    /// Loops that were present but not synced, with reasons.
    pub skipped: Vec<LoopSyncSkip>,
    /// Non-fatal authoring warnings, including ignored Git authority fields.
    pub warnings: Vec<LoopSyncWarning>,
}

/// Response for `POST /api/v1/repositories/{id}/sync-config`.
///
/// Wraps the updated repository plus reconciliation summaries; this explicit
/// sync also reclaims manually overridden workspaces — see
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

    use super::{UpdateLoopConfig, UpdateSubscription};

    #[test]
    fn update_loop_config_distinguishes_omitted_null_and_value() {
        let omitted: UpdateLoopConfig = serde_json::from_value(json!({})).unwrap();
        assert_eq!(omitted.model, None);

        let cleared: UpdateLoopConfig = serde_json::from_value(json!({ "model": null })).unwrap();
        assert_eq!(cleared.model, Some(None));

        let set: UpdateLoopConfig = serde_json::from_value(json!({ "model": "fable" })).unwrap();
        assert_eq!(set.model, Some(Some("fable".to_string())));
    }

    /// `ongoing_task_id`, `task_name`, `prompt` and `close_after_settled_days` all
    /// carry the same tri-state attribute as `model`; an explicit `null` must
    /// reach the server as `Some(None)`, not collapse into "omitted".
    #[test]
    fn update_loop_config_clearable_fields_distinguish_omitted_and_null() {
        let omitted: UpdateLoopConfig = serde_json::from_value(json!({})).unwrap();
        assert_eq!(omitted.ongoing_task_id, None);
        assert_eq!(omitted.task_name, None);
        assert_eq!(omitted.prompt, None);
        assert_eq!(omitted.close_after_settled_days, None);

        let cleared: UpdateLoopConfig = serde_json::from_value(json!({
            "ongoingTaskId": null,
            "taskName": null,
            "prompt": null,
            "closeAfterSettledDays": null,
        }))
        .unwrap();
        assert_eq!(cleared.ongoing_task_id, Some(None));
        assert_eq!(cleared.task_name, Some(None));
        assert_eq!(cleared.prompt, Some(None));
        assert_eq!(cleared.close_after_settled_days, Some(None));

        let set: UpdateLoopConfig = serde_json::from_value(json!({
            "ongoingTaskId": "task-1",
            "taskName": "name",
            "prompt": "do it",
            "closeAfterSettledDays": 7,
        }))
        .unwrap();
        assert_eq!(set.ongoing_task_id, Some(Some("task-1".to_string())));
        assert_eq!(set.task_name, Some(Some("name".to_string())));
        assert_eq!(set.prompt, Some(Some("do it".to_string())));
        assert_eq!(set.close_after_settled_days, Some(Some(7)));
    }

    /// `source_name` and `adapter_config` carry the same tri-state attribute;
    /// an explicit `null` must reach the server as `Some(None)`.
    #[test]
    fn update_subscription_clearable_fields_distinguish_omitted_and_null() {
        let omitted: UpdateSubscription = serde_json::from_value(json!({})).unwrap();
        assert_eq!(omitted.source_name, None);
        assert_eq!(omitted.adapter_config, None);

        let cleared: UpdateSubscription = serde_json::from_value(json!({
            "sourceName": null,
            "adapterConfig": null,
        }))
        .unwrap();
        assert_eq!(cleared.source_name, Some(None));
        assert_eq!(cleared.adapter_config, Some(None));

        let set: UpdateSubscription = serde_json::from_value(json!({
            "sourceName": "acme",
            "adapterConfig": {"a": 1},
        }))
        .unwrap();
        assert_eq!(set.source_name, Some(Some("acme".to_string())));
        assert_eq!(set.adapter_config, Some(Some(json!({"a": 1}))));
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
