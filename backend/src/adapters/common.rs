use cron::Schedule as CronSchedule;
use moatless_api_schemas::adapters::AdapterConnectionResponse;
use moatless_api_schemas::bindings::{ReplyTargetDescriptor, TaskBindingResponse};
use moatless_api_schemas::enums::{AdapterKind, Scope};
use moatless_api_schemas::loops::{
    Loop, LoopConfig, LoopKind, LoopSource, ManualSource, Schedule, Subscription,
};
use serde::Deserialize;

use crate::access_control::{
    ensure_can_use_repository, ensure_can_write_resource, resolve_scope_for_create, ResolvedScope,
};
use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, TaskBinding};
use crate::extractors::AuthUser;
use crate::loops::store::{LoopRecord, LoopSourceRow};
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

pub(crate) fn parse_adapter_kind(provider: &str) -> AdapterKind {
    AdapterKind::parse(provider).unwrap_or(AdapterKind::Webhook)
}

pub(crate) fn default_scope(scope: Option<Scope>, team_id: Option<&str>) -> Scope {
    scope.unwrap_or(if team_id.is_some() {
        Scope::Team
    } else {
        Scope::User
    })
}

pub(crate) fn adapter_connection_response(row: &AdapterConnection) -> AdapterConnectionResponse {
    AdapterConnectionResponse {
        id: row.id.clone(),
        adapter_kind: parse_adapter_kind(&row.provider),
        connection_kind: row.connection_kind.clone(),
        external_account_id: row.scope_value.clone(),
        created_by: row.created_by.clone(),
        scope: row.scope,
        team_id: row.team_id.clone(),
        owner_user_id: row.owner_user_id.clone(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

pub(crate) fn loop_response(record: &LoopRecord) -> Loop {
    let kind = LoopKind::try_from(record.trigger.kind.as_str()).unwrap_or(LoopKind::Manual);
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
        id: record.trigger.id.clone(),
        name: record.trigger.name.clone(),
        kind,
        active: record.trigger.active,
        scope: record.trigger.scope,
        team_id: record.trigger.team_id.clone(),
        owner_user_id: record.trigger.owner_user_id.clone(),
        created_by: record.trigger.created_by.clone(),
        created_at: record.trigger.created_at.to_rfc3339(),
        updated_at: record.trigger.updated_at.to_rfc3339(),
        config: LoopConfig {
            repository_id: record.rule.repository_id.clone(),
            routing_mode: record.rule.routing_mode,
            ongoing_task_id: record.rule.ongoing_task_id.clone(),
            task_name: record.rule.task_name.clone(),
            prompt: record.rule.prompt.clone(),
            tag_ids: record.rule.tag_ids.clone(),
            run_as_user_id: record.rule.run_as_user_id.clone(),
            agent_type: record.rule.agent_type.clone(),
            inactive_ttl_days: record.rule.inactive_ttl_days,
        },
        source,
    }
}

pub(crate) fn task_binding_response(row: &TaskBinding) -> TaskBindingResponse {
    TaskBindingResponse {
        id: row.id.clone(),
        task_id: row.task_id.clone(),
        adapter_kind: parse_adapter_kind(&row.provider),
        connection_id: row.connection_id.clone(),
        trigger_id: row.trigger_id.clone(),
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
        .find_repository_by_id(repository_id)
        .await
        .map_err(|e| AdapterError::Internal(format!("Failed to look up repository: {e}")))?
        .ok_or_else(|| AdapterError::BadRequest(format!("Repository {repository_id} not found")))?;
    ensure_scope_access(
        ctx,
        repo.scope,
        repo.team_id.as_deref(),
        repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
        "repository",
        repository_id,
    )?;
    ensure_can_use_repository(
        ctx,
        &ScopedResource::borrowed(
            repo.scope,
            repo.team_id.as_deref(),
            repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
        ),
    )?;
    Ok(())
}

pub(crate) fn ensure_scope_access(
    ctx: &ScopeContext,
    scope: Scope,
    team_id: Option<&str>,
    owner_id: Option<&str>,
    resource_kind: &str,
    resource_id: &str,
) -> Result<(), AdapterError> {
    if ctx.can_access(scope, team_id, owner_id) {
        Ok(())
    } else {
        Err(AdapterError::AccessDenied(format!(
            "No access to {resource_kind} {resource_id}"
        )))
    }
}

pub(crate) fn scoped_resource(
    scope: Scope,
    team_id: Option<&str>,
    owner_user_id: Option<&str>,
    created_by: &str,
) -> ScopedResource {
    ScopedResource::borrowed(scope, team_id, owner_user_id.or(Some(created_by)))
}

pub(crate) enum TeamIdUpdate {
    Unchanged,
    Set(Option<String>),
}

pub(crate) async fn resolve_scope_for_adapter_create(
    state: &AppState,
    auth: &AuthUser,
    requested_scope: Option<Scope>,
    requested_team_id: Option<String>,
) -> Result<ResolvedScope, AdapterError> {
    let scope = default_scope(requested_scope, requested_team_id.as_deref());
    Ok(resolve_scope_for_create(state, auth, Some(scope), requested_team_id).await?)
}

pub(crate) async fn resolve_scope_for_adapter_update(
    state: &AppState,
    auth: &AuthUser,
    requested_scope: Option<Scope>,
    requested_team_id: TeamIdUpdate,
    current: &ScopedResource,
) -> Result<ResolvedScope, AdapterError> {
    if requested_scope.is_none() && matches!(requested_team_id, TeamIdUpdate::Unchanged) {
        let Some(team_id) = current.team_id.clone() else {
            return Err(AdapterError::BadRequest(
                "teamId is required for scoped adapter resources".into(),
            ));
        };
        return Ok(ResolvedScope {
            scope: current.scope,
            team_id,
            owner_user_id: current.owner_user_id.clone(),
        });
    }

    let next_scope = requested_scope.unwrap_or(current.scope);
    let next_team_id = match requested_team_id {
        TeamIdUpdate::Set(team_id) => team_id,
        TeamIdUpdate::Unchanged => current.team_id.clone(),
    };
    Ok(resolve_scope_for_create(state, auth, Some(next_scope), next_team_id).await?)
}

pub(crate) async fn ensure_can_write_adapter_resource(
    state: &AppState,
    auth: &AuthUser,
    resource: &ScopedResource,
    resource_name: &str,
) -> Result<(), AdapterError> {
    Ok(ensure_can_write_resource(state, auth, resource, resource_name).await?)
}

/// Authorize a `run_as_user_id` choice for a trigger.
///
/// Allows the choice when:
/// 1. The target is the caller themselves, or the caller is an admin
///    (existing `ctx.can_run_as` rule), or
/// 2. The target is a bot user (`users.is_bot = true`). For v1 there is no
///    team scoping on bots — any authenticated human user can point a
///    trigger at any bot.
///
/// Bot-on-bot impersonation is blocked: a caller who is themselves a bot
/// (`ctx.is_bot = true`) cannot pick a different bot as the `run_as_user_id`
/// even though `ctx.can_run_as` would otherwise allow it for admins.
/// Today no caller path constructs a `ScopeContext` for a bot user, so this
/// guard is defensive; it removes transitive impersonation as a follow-up
/// risk without requiring changes elsewhere.
pub(crate) async fn ensure_can_run_as(
    state: &AppState,
    ctx: &ScopeContext,
    run_as_user_id: Option<&str>,
) -> Result<(), AdapterError> {
    let Some(target) = run_as_user_id else {
        return Ok(());
    };
    if ctx.can_run_as(target) {
        return Ok(());
    }
    // `target` is a uuid; a legacy provider id also resolves via the identity map.
    let found = match target.parse::<uuid::Uuid>() {
        Ok(uid) => state.users.get_user(&uid).await,
        Err(_) => state.users.find_user_by_legacy_id(target).await,
    };
    match found {
        Ok(Some(user)) if user.is_bot => Ok(()),
        Ok(_) => Err(AdapterError::AccessDenied(format!(
            "Cannot run as user {target}"
        ))),
        Err(e) => Err(AdapterError::Internal(format!(
            "Failed to look up run-as user {target}: {e}"
        ))),
    }
}

pub(crate) async fn load_adapter_connection_for_auth(
    state: &AppState,
    ctx: &ScopeContext,
    id: &str,
) -> Result<AdapterConnection, AdapterError> {
    let row = PgAdapterConnectionRepository(state.db.clone())
        .find_by_id(id)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Provider connection {id} not found")))?;
    ensure_scope_access(
        ctx,
        row.scope,
        row.team_id.as_deref(),
        row.owner_user_id
            .as_deref()
            .or(Some(row.created_by.as_str())),
        "provider connection",
        id,
    )?;
    Ok(row)
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
        if conn.provider != kind.as_str() {
            return Err(AdapterError::BadRequest(format!(
                "Connection {id} is for provider {}, not {}",
                conn.provider,
                kind.as_str()
            )));
        }
        return Ok(conn);
    }
    let rows = PgAdapterConnectionRepository(state.db.clone())
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

pub(crate) async fn load_loop_for_auth(
    state: &AppState,
    ctx: &ScopeContext,
    id: &str,
) -> Result<LoopRecord, AdapterError> {
    let record = state
        .loops
        .get(id)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Trigger {id} not found")))?;
    ensure_scope_access(
        ctx,
        record.trigger.scope,
        record.trigger.team_id.as_deref(),
        record
            .trigger
            .owner_user_id
            .as_deref()
            .or(Some(record.trigger.created_by.as_str())),
        "trigger",
        id,
    )?;
    Ok(record)
}

pub(crate) async fn ensure_task_access(
    state: &AppState,
    ctx: &ScopeContext,
    task_id: &str,
) -> Result<crate::task::models::Task, AdapterError> {
    let task = state
        .tasks
        .find_by_id_internal(task_id)
        .await
        .map_err(|e| AdapterError::Internal(format!("Failed to look up task: {e}")))?
        .ok_or_else(|| AdapterError::NotFound(format!("Task {task_id} not found")))?;
    ensure_scope_access(
        ctx,
        task.scope,
        task.team_id.as_deref(),
        task.owner_user_id.as_deref().or(task.created_by.as_deref()),
        "task",
        task_id,
    )?;
    Ok(task)
}
