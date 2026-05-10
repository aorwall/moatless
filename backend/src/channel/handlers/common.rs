use moatless_api_schemas::channel::{
    ChannelTargetDescriptor, ProviderConnectionResponse, ScheduledJobResponse, TaskBindingResponse,
    TaskRouteResponse, TaskTriggerResponse,
};
use moatless_api_schemas::enums::{IntegrationProvider, Scope};
use serde::Deserialize;

use crate::access_control::{
    ensure_can_use_repository, ensure_can_write_resource, resolve_scope_for_create, ResolvedScope,
};
use crate::channel::dao::connections::PgProviderConnectionRepository;
use crate::channel::dao::routes::PgTaskRouteRepository;
use crate::channel::dao::triggers::PgTaskTriggerRepository;
use crate::channel::error::ChannelError;
use crate::channel::models::{
    ProviderConnection, ScheduledJob, TaskBinding, TaskRoute, TaskTrigger,
};
use crate::channel::services::scheduled_jobs::ScheduledJobService;
use crate::extractors::AuthUser;
use crate::scope::{ScopeContext, ScopedResource};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFilter {
    pub provider: Option<IntegrationProvider>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteFilter {
    pub repository_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerFilter {
    pub provider: Option<IntegrationProvider>,
    pub route_id: Option<String>,
}

pub(crate) fn parse_provider(provider: &str) -> IntegrationProvider {
    IntegrationProvider::parse(provider).unwrap_or(IntegrationProvider::Webhook)
}

pub(crate) fn default_scope(scope: Option<Scope>, team_id: Option<&str>) -> Scope {
    scope.unwrap_or(if team_id.is_some() {
        Scope::Team
    } else {
        Scope::User
    })
}

pub(crate) fn provider_connection_response(row: &ProviderConnection) -> ProviderConnectionResponse {
    ProviderConnectionResponse {
        id: row.id.clone(),
        provider: parse_provider(&row.provider),
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

pub(crate) fn task_route_response(row: &TaskRoute) -> TaskRouteResponse {
    TaskRouteResponse {
        id: row.id.clone(),
        repository_id: row.repository_id.clone(),
        routing_mode: row.routing_mode,
        ongoing_task_id: row.ongoing_task_id.clone(),
        prompt_template: row.prompt_template.clone(),
        task_name: row.task_name.clone(),
        tag_ids: row.tag_ids.clone(),
        run_as_user_id: row.run_as_user_id.clone(),
        agent_type: row.agent_type.clone(),
        scope: row.scope,
        team_id: row.team_id.clone(),
        owner_user_id: row.owner_user_id.clone(),
        created_by: row.created_by.clone(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

pub(crate) fn task_trigger_response(row: &TaskTrigger) -> TaskTriggerResponse {
    TaskTriggerResponse {
        id: row.id.clone(),
        provider: parse_provider(&row.provider),
        connection_id: row.connection_id.clone(),
        source_matcher: row.source_matcher.clone(),
        source_name: row.source_name.clone(),
        event_filter: row.event_filter.clone(),
        trigger_config: row.trigger_config.clone(),
        route_id: row.route_id.clone(),
        allow_direct_send: row.allow_direct_send,
        close_task_on_close: row.close_task_on_close,
        notify_on_success: row.notify_on_success,
        active: row.active,
        scope: row.scope,
        team_id: row.team_id.clone(),
        owner_user_id: row.owner_user_id.clone(),
        created_by: row.created_by.clone(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

pub(crate) fn task_binding_response(row: &TaskBinding) -> TaskBindingResponse {
    TaskBindingResponse {
        id: row.id.clone(),
        task_id: row.task_id.clone(),
        provider: parse_provider(&row.provider),
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

pub(crate) fn scheduled_job_response(row: &ScheduledJob) -> ScheduledJobResponse {
    ScheduledJobResponse {
        id: row.id.clone(),
        route_id: row.route_id.clone(),
        schedule_expr: row.schedule_expr.clone(),
        timezone: row.timezone.clone(),
        message_template: row.message_template.clone(),
        run_as_user_id: row.run_as_user_id.clone(),
        active: row.active,
        last_fired_at: row.last_fired_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: row.next_fire_at.to_rfc3339(),
        locked_by: row.locked_by.clone(),
        locked_until: row.locked_until.map(|dt| dt.to_rfc3339()),
        created_by: row.created_by.clone(),
        scope: row.scope,
        team_id: row.team_id.clone(),
        owner_user_id: row.owner_user_id.clone(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

pub(crate) fn binding_descriptor(binding: &TaskBinding) -> ChannelTargetDescriptor {
    ChannelTargetDescriptor {
        kind: "binding".to_string(),
        id: binding.id.clone(),
        label: format!("{} {}", binding.subject_kind, binding.subject_key),
        flag_hint: format!("--binding {}", binding.id),
    }
}

pub(crate) fn next_fire_at(
    schedule_expr: &str,
) -> Result<chrono::DateTime<chrono::Utc>, ChannelError> {
    ScheduledJobService::next_fire_at(schedule_expr)
}

pub(crate) async fn ensure_repository_exists(
    state: &AppState,
    ctx: &ScopeContext,
    repository_id: &str,
) -> Result<(), ChannelError> {
    let repo = state
        .repositories
        .find_repository_by_id(repository_id)
        .await
        .map_err(|e| ChannelError::Internal(format!("Failed to look up repository: {e}")))?
        .ok_or_else(|| ChannelError::BadRequest(format!("Repository {repository_id} not found")))?;
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
) -> Result<(), ChannelError> {
    if ctx.can_access(scope, team_id, owner_id) {
        Ok(())
    } else {
        Err(ChannelError::AccessDenied(format!(
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

pub(crate) async fn resolve_scope_for_channel_create(
    state: &AppState,
    auth: &AuthUser,
    requested_scope: Option<Scope>,
    requested_team_id: Option<String>,
) -> Result<ResolvedScope, ChannelError> {
    let scope = default_scope(requested_scope, requested_team_id.as_deref());
    Ok(resolve_scope_for_create(state, auth, Some(scope), requested_team_id).await?)
}

pub(crate) async fn resolve_scope_for_channel_update(
    state: &AppState,
    auth: &AuthUser,
    requested_scope: Option<Scope>,
    requested_team_id: TeamIdUpdate,
    current: &ScopedResource,
) -> Result<ResolvedScope, ChannelError> {
    if requested_scope.is_none() && matches!(requested_team_id, TeamIdUpdate::Unchanged) {
        let Some(team_id) = current.team_id.clone() else {
            return Err(ChannelError::BadRequest(
                "teamId is required for scoped integration resources".into(),
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

pub(crate) async fn ensure_can_write_channel_resource(
    state: &AppState,
    auth: &AuthUser,
    resource: &ScopedResource,
    resource_name: &str,
) -> Result<(), ChannelError> {
    Ok(ensure_can_write_resource(state, auth, resource, resource_name).await?)
}

pub(crate) fn ensure_can_run_as(
    ctx: &ScopeContext,
    run_as_user_id: Option<&str>,
) -> Result<(), ChannelError> {
    if let Some(run_as_user_id) = run_as_user_id {
        if !ctx.can_run_as(run_as_user_id) {
            return Err(ChannelError::AccessDenied(format!(
                "Cannot run as user {run_as_user_id}"
            )));
        }
    }
    Ok(())
}

pub(crate) async fn load_provider_connection_for_auth(
    state: &AppState,
    ctx: &ScopeContext,
    id: &str,
) -> Result<ProviderConnection, ChannelError> {
    let row = PgProviderConnectionRepository(state.db.clone())
        .find_by_id(id)
        .await?
        .ok_or_else(|| ChannelError::NotFound(format!("Provider connection {id} not found")))?;
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

pub(crate) async fn load_task_route_for_auth(
    state: &AppState,
    ctx: &ScopeContext,
    id: &str,
) -> Result<TaskRoute, ChannelError> {
    let row = PgTaskRouteRepository(state.db.clone())
        .find_by_id(id)
        .await?
        .ok_or_else(|| ChannelError::NotFound(format!("Task route {id} not found")))?;
    ensure_scope_access(
        ctx,
        row.scope,
        row.team_id.as_deref(),
        row.owner_user_id
            .as_deref()
            .or(Some(row.created_by.as_str())),
        "task route",
        id,
    )?;
    Ok(row)
}

pub(crate) async fn load_task_trigger_for_auth(
    state: &AppState,
    ctx: &ScopeContext,
    id: &str,
) -> Result<TaskTrigger, ChannelError> {
    let row = PgTaskTriggerRepository(state.db.clone())
        .find_by_id(id)
        .await?
        .ok_or_else(|| ChannelError::NotFound(format!("Task trigger {id} not found")))?;
    ensure_scope_access(
        ctx,
        row.scope,
        row.team_id.as_deref(),
        row.owner_user_id
            .as_deref()
            .or(Some(row.created_by.as_str())),
        "task trigger",
        id,
    )?;
    Ok(row)
}

pub(crate) async fn load_scheduled_job_for_auth(
    state: &AppState,
    ctx: &ScopeContext,
    id: &str,
) -> Result<ScheduledJob, ChannelError> {
    let row = sqlx::query_as::<_, ScheduledJob>("SELECT * FROM scheduled_jobs WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ChannelError::NotFound(format!("Scheduled job {id} not found")))?;
    ensure_scope_access(
        ctx,
        row.scope,
        row.team_id.as_deref(),
        row.owner_user_id
            .as_deref()
            .or(Some(row.created_by.as_str())),
        "scheduled job",
        id,
    )?;
    Ok(row)
}

pub(crate) async fn ensure_task_access(
    state: &AppState,
    ctx: &ScopeContext,
    task_id: &str,
) -> Result<crate::task::models::Task, ChannelError> {
    let task = state
        .tasks
        .find_by_id_internal(task_id)
        .await
        .map_err(|e| ChannelError::Internal(format!("Failed to look up task: {e}")))?
        .ok_or_else(|| ChannelError::NotFound(format!("Task {task_id} not found")))?;
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
