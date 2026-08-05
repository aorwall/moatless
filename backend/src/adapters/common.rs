use cron::Schedule as CronSchedule;
use moatless_api_schemas::adapters::AdapterConnectionResponse;
use moatless_api_schemas::bindings::{ReplyTargetDescriptor, TaskBindingResponse};
use moatless_api_schemas::enums::{AdapterKind, Scope};
use moatless_api_schemas::loops::{
    Loop, LoopConfig, LoopConfigSource, LoopExecutionState, LoopKind, LoopSource, ManualSource,
    Schedule, Subscription,
};
use serde::Deserialize;

use crate::access_control::{ensure_can_use_repository, resolve_scope_for_create, ResolvedScope};
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, TaskBinding};
// `parse_adapter_kind` now lives in `moatless-adapter-model`; re-exported so the
// handler-support callers here and in `loops`/`task` keep their path unchanged.
pub(crate) use crate::adapters::models::parse_adapter_kind;
use crate::loops::dao::{LoopRecord, LoopSourceRow};
use crate::state::AppState;
use moatless_backend_core::scope::{ScopeContext, ScopedResource};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterFilter {
    pub adapter_kind: Option<AdapterKind>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopFilter {
    pub kind: Option<LoopKind>,
    pub adapter_kind: Option<AdapterKind>,
}

pub(crate) fn adapter_connection_response(row: &AdapterConnection) -> AdapterConnectionResponse {
    AdapterConnectionResponse {
        id: row.id.clone(),
        adapter_kind: parse_adapter_kind(&row.adapter_kind),
        connection_kind: row.connection_kind.clone(),
        external_account_id: row.scope_value.clone(),
        created_by: row.created_by.clone(),
        scope: row.scope,
        owner_user_id: row.owner_user_id.clone(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

pub(crate) fn loop_response(record: &LoopRecord) -> Loop {
    let kind = LoopKind::try_from(record.loop_.kind.as_str()).unwrap_or(LoopKind::Manual);
    let config_source = LoopConfigSource::try_from(record.loop_.config_source.as_str())
        .unwrap_or(LoopConfigSource::Manual);
    let execution_state = if record.loop_.active {
        LoopExecutionState::Active
    } else if record.config.run_as_user_id.is_some() {
        LoopExecutionState::Paused
    } else {
        LoopExecutionState::AwaitingApproval
    };
    let source = match &record.source {
        LoopSourceRow::AdapterEvent(source) => LoopSource::AdapterEvent(Subscription {
            adapter_kind: parse_adapter_kind(&source.adapter_kind),
            connection_id: source.connection_id.clone(),
            source_matcher: source.source_matcher.clone(),
            source_name: source.source_name.clone(),
            event_filter: source.event_filter.clone(),
            adapter_config: source.adapter_config.clone(),
            allow_direct_send: source.allow_direct_send,
            close_task_on_close: source.close_task_on_close,
            notify_on_success: source.notify_on_success,
        }),
        LoopSourceRow::Schedule(source) => LoopSource::Schedule(Schedule {
            cron_expression: source.cron_expression.clone(),
            timezone: source.timezone.clone(),
            message_template: source.message_template.clone(),
            last_fired_at: source.last_fired_at.map(|dt| dt.to_rfc3339()),
            next_fire_at: source.next_fire_at.to_rfc3339(),
            locked_by: source.locked_by.clone(),
            locked_until: source.locked_until.map(|dt| dt.to_rfc3339()),
        }),
        LoopSourceRow::Manual => LoopSource::Manual(ManualSource {}),
    };

    Loop {
        id: record.loop_.id.clone(),
        name: record.loop_.name.clone(),
        kind,
        active: record.loop_.active,
        scope: record.loop_.scope,
        owner_user_id: record.loop_.owner_user_id.clone(),
        created_by: record.loop_.created_by.clone(),
        created_at: record.loop_.created_at.to_rfc3339(),
        updated_at: record.loop_.updated_at.to_rfc3339(),
        config_source,
        synced_from_git: config_source == LoopConfigSource::Git && !record.loop_.deleted,
        source_repository_id: record.loop_.source_repository_id.clone(),
        source_key: record.loop_.source_key.clone(),
        source_config_path: record.loop_.source_config_path.clone(),
        deleted: record.loop_.deleted,
        execution_state,
        config: LoopConfig {
            repository_id: record.config.repository_id.clone(),
            routing_mode: record.config.routing_mode,
            ongoing_task_id: record.config.ongoing_task_id.clone(),
            task_name: record.config.task_name.clone(),
            prompt: record.config.prompt.clone(),
            tag_ids: record.config.tag_ids.clone(),
            run_as_user_id: record.config.run_as_user_id.clone(),
            agent_type: record.config.agent_type.clone(),
            inactive_ttl_days: record.config.inactive_ttl_days,
        },
        source,
    }
}

pub(crate) fn task_binding_response(row: &TaskBinding) -> TaskBindingResponse {
    TaskBindingResponse {
        id: row.id.clone(),
        task_id: row.task_id.clone(),
        adapter_kind: parse_adapter_kind(&row.adapter_kind),
        connection_id: row.connection_id.clone(),
        loop_id: row.loop_id.clone(),
        subject_kind: row.subject_kind.clone(),
        subject_key: row.subject_key.clone(),
        destination_config: row.destination_config.clone(),
        metadata: row.metadata.clone(),
        last_external_event_id: row.last_external_event_id.clone(),
        origin: row.origin.clone(),
        created_at: row.created_at.to_rfc3339(),
        last_event_at: row.last_event_at.to_rfc3339(),
    }
}

pub(crate) fn binding_descriptor(binding: &TaskBinding) -> ReplyTargetDescriptor {
    ReplyTargetDescriptor {
        kind: "binding".to_string(),
        id: binding.id.clone(),
        label: format!("{} {}", binding.subject_kind, binding.subject_key),
        flag_hint: format!("--binding {}", binding.id),
    }
}

pub(crate) fn next_fire_at(
    schedule_expr: &str,
) -> Result<chrono::DateTime<chrono::Utc>, AdapterError> {
    let schedule: CronSchedule = schedule_expr
        .parse()
        .map_err(|e| AdapterError::BadRequest(format!("Invalid cron expression: {e}")))?;
    schedule
        .after(&chrono::Utc::now())
        .next()
        .ok_or_else(|| AdapterError::BadRequest("Schedule has no future fire time".to_string()))
}

pub(crate) async fn ensure_repository_exists(
    state: &AppState,
    ctx: &ScopeContext,
    repository_id: &str,
) -> Result<(), AdapterError> {
    let repo = state
        .repositories
        .find_by_id(repository_id, ctx)
        .await?
        .ok_or_else(|| AdapterError::BadRequest(format!("Repository {repository_id} not found")))?;
    ensure_can_use_repository(
        ctx,
        &ScopedResource::borrowed(
            repo.scope,
            repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
        ),
    )?;
    Ok(())
}

/// Resolve the scope for a new adapter resource. Defaults to `User` owned by
/// the caller; requesting `Global` requires admin.
pub(crate) fn resolve_scope_for_adapter_create(
    ctx: &ScopeContext,
    requested_scope: Option<Scope>,
) -> Result<ResolvedScope, AdapterError> {
    Ok(resolve_scope_for_create(ctx, requested_scope)?)
}

/// Load and authorize an adapter connection for the calling user.
///
/// Uses the scoped `find_by_id(id, ctx)` DAO method so only connections
/// visible to the caller are returned; `None` → `NotFound`.
pub(crate) async fn load_adapter_connection_for_auth(
    state: &AppState,
    ctx: &ScopeContext,
    id: &str,
) -> Result<AdapterConnection, AdapterError> {
    state
        .adapter_connections
        .find_by_id(id, ctx)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Provider connection {id} not found")))
}

/// Load and authorize a loop for the calling user.
///
/// Uses the scoped `get(id, ctx)` store method so only loops visible to the
/// caller are returned; `None` → `NotFound`.
pub(crate) async fn load_loop_for_auth(
    state: &AppState,
    ctx: &ScopeContext,
    id: &str,
) -> Result<LoopRecord, AdapterError> {
    state
        .loops
        .get(id, ctx)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Loop {id} not found")))
}

/// Load and authorize a task for the calling user.
///
/// Uses the scoped `find_by_id(id, ctx)` DAO method; `None` → `NotFound`.
pub(crate) async fn ensure_task_access(
    state: &AppState,
    ctx: &ScopeContext,
    task_id: &str,
) -> Result<crate::task::models::Task, AdapterError> {
    state
        .tasks
        .find_by_id(task_id, ctx)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Task {task_id} not found")))
}

/// Resolve an adapter connection for the caller.
///
/// - `Some(id)` → load and authorize that specific connection.
/// - `None` → find all `kind` connections accessible to `ctx`:
///   * 0 → `BadRequest` "no <kind> connection configured"
///   * 1 → return it
///   * 2+ → `BadRequest` with the list of accessible IDs and a hint
pub(crate) async fn resolve_connection(
    state: &AppState,
    ctx: &ScopeContext,
    kind: AdapterKind,
    connection_id: Option<&str>,
) -> Result<AdapterConnection, AdapterError> {
    if let Some(id) = connection_id {
        let conn = load_adapter_connection_for_auth(state, ctx, id).await?;
        if conn.adapter_kind != kind.as_str() {
            return Err(AdapterError::BadRequest(format!(
                "Connection {id} is for adapter {}, not {}",
                conn.adapter_kind,
                kind.as_str()
            )));
        }
        return Ok(conn);
    }
    let rows = state
        .adapter_connections
        .list(ctx, Some(kind.as_str()))
        .await?;
    let mut iter = rows.into_iter();
    match (iter.next(), iter.next()) {
        (None, _) => Err(AdapterError::BadRequest(format!(
            "No {} connection configured. Create one with `moat adapters connections create --adapter {} ...` first.",
            kind.as_str(),
            kind.as_str()
        ))),
        (Some(only), None) => Ok(only),
        (Some(first), Some(second)) => {
            let mut ids: Vec<String> = vec![first.id, second.id];
            for row in iter {
                ids.push(row.id);
            }
            Err(AdapterError::BadRequest(format!(
                "Multiple {} connections accessible: [{}]. Pass --connection <id> to choose.",
                kind.as_str(),
                ids.join(", ")
            )))
        }
    }
}
