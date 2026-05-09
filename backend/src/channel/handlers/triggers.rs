use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::channel::{
    CreateTaskTriggerRequest, TaskBindingResponse, TaskTriggerResponse, TriggerRecentTaskItem,
    TriggerRecentTaskSummary, UpdateTaskTriggerRequest,
};
use moatless_api_schemas::enums::TaskStatus;
use serde::Deserialize;
use utoipa::ToSchema;

use super::common::{
    default_scope, ensure_can_write_scope, ensure_repository_exists,
    load_provider_connection_for_auth, load_task_route_for_auth, load_task_trigger_for_auth,
    parse_provider, task_trigger_response, TriggerFilter,
};
use crate::channel::dao::bindings::{PgTaskBindingRepository, TriggerRecentTaskRow};
use crate::channel::dao::triggers::PgTaskTriggerRepository;
use crate::channel::error::ChannelError;
use crate::channel::models::TaskTrigger;
use crate::extractors::AuthUser;
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RecentTasksQuery {
    pub limit: Option<u32>,
}

fn map_recent_task_item(row: &TriggerRecentTaskRow) -> TriggerRecentTaskItem {
    let status = TaskStatus::try_from(row.status.as_str()).unwrap_or(TaskStatus::Open);
    let binding = TaskBindingResponse {
        id: row.binding_id.clone(),
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
        created_at: row.binding_created_at.to_rfc3339(),
        last_event_at: row.last_event_at.to_rfc3339(),
    };
    TriggerRecentTaskItem {
        task: TriggerRecentTaskSummary {
            task_id: row.task_id.clone(),
            short_id: row.short_id.clone(),
            title: row.title.clone(),
            status,
            created_at: row.task_created_at.to_rfc3339(),
        },
        binding,
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/triggers",
    tag = "triggers",
    responses((status = 200, description = "Task triggers", body = Vec<TaskTriggerResponse>))
)]
pub async fn list_task_triggers(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<TriggerFilter>,
) -> Result<Json<Vec<TaskTriggerResponse>>, ChannelError> {
    let ctx = auth.scope_context();
    let provider = query.provider.map(|p| p.as_str().to_string());
    let rows = PgTaskTriggerRepository(state.db.clone())
        .list(&ctx, provider.as_deref(), query.route_id.as_deref())
        .await?;
    Ok(Json(rows.iter().map(task_trigger_response).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/triggers",
    tag = "triggers",
    request_body = CreateTaskTriggerRequest,
    responses((status = 200, description = "Task trigger created", body = TaskTriggerResponse))
)]
pub async fn create_task_trigger(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateTaskTriggerRequest>,
) -> Result<Json<TaskTriggerResponse>, ChannelError> {
    let ctx = auth.scope_context();
    let provider = req.provider.as_str().to_string();
    let connection = load_provider_connection_for_auth(&state, &ctx, &req.connection_id).await?;
    if connection.provider != provider {
        return Err(ChannelError::BadRequest(format!(
            "Connection {} is for provider {}, not {}",
            connection.id, connection.provider, provider
        )));
    }
    let route = load_task_route_for_auth(&state, &ctx, &req.route_id).await?;
    ensure_repository_exists(&state, &ctx, &route.repository_id).await?;
    let scope = default_scope(req.scope, req.team_id.as_deref());
    ensure_can_write_scope(&ctx, scope, req.team_id.as_deref())?;

    let now = Utc::now();
    let row = sqlx::query_as::<_, TaskTrigger>(
        r#"INSERT INTO task_triggers
           (id, provider, connection_id, source_matcher, source_name, event_filter,
            trigger_config, route_id, allow_direct_send,
            close_task_on_close, notify_on_success, active, scope, team_id,
            created_by, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                   $10, $11, $12, $13, $14, $15, $16, $17)
           RETURNING *"#,
    )
    .bind(format!("tt_{}", uuid::Uuid::new_v4()))
    .bind(provider)
    .bind(req.connection_id)
    .bind(req.source_matcher)
    .bind(req.source_name)
    .bind(req.event_filter.unwrap_or_else(|| serde_json::json!([])))
    .bind(req.trigger_config)
    .bind(req.route_id)
    .bind(req.allow_direct_send.unwrap_or(false))
    .bind(req.close_task_on_close.unwrap_or(true))
    .bind(req.notify_on_success.unwrap_or(false))
    .bind(req.active.unwrap_or(true))
    .bind(scope)
    .bind(req.team_id)
    .bind(auth.user_id)
    .bind(now)
    .bind(now)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(task_trigger_response(&row)))
}

#[utoipa::path(
    put,
    path = "/api/v1/triggers/{id}",
    tag = "triggers",
    request_body = UpdateTaskTriggerRequest,
    responses((status = 200, description = "Task trigger updated", body = TaskTriggerResponse))
)]
pub async fn update_task_trigger(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateTaskTriggerRequest>,
) -> Result<Json<TaskTriggerResponse>, ChannelError> {
    let ctx = auth.scope_context();
    let current = load_task_trigger_for_auth(&state, &ctx, &id).await?;
    let next_connection_id = req
        .connection_id
        .clone()
        .unwrap_or_else(|| current.connection_id.clone());
    let connection = load_provider_connection_for_auth(&state, &ctx, &next_connection_id).await?;
    if connection.provider != current.provider {
        return Err(ChannelError::BadRequest(format!(
            "Connection {} is for provider {}, not {}",
            connection.id, connection.provider, current.provider
        )));
    }
    let next_route_id = req
        .route_id
        .clone()
        .unwrap_or_else(|| current.route_id.clone());
    let next_route = load_task_route_for_auth(&state, &ctx, &next_route_id).await?;
    ensure_repository_exists(&state, &ctx, &next_route.repository_id).await?;
    let next_scope = req.scope.unwrap_or(current.scope);
    let next_team_id = req
        .team_id
        .clone()
        .unwrap_or_else(|| current.team_id.clone());
    ensure_can_write_scope(&ctx, next_scope, next_team_id.as_deref())?;

    let row = sqlx::query_as::<_, TaskTrigger>(
        r#"UPDATE task_triggers
           SET connection_id = $2,
               source_matcher = $3,
               source_name = $4,
               event_filter = $5,
               trigger_config = $6,
               route_id = $7,
               allow_direct_send = $8,
               close_task_on_close = $9,
               notify_on_success = $10,
               active = $11,
               scope = $12,
               team_id = $13,
               updated_at = now()
           WHERE id = $1
           RETURNING *"#,
    )
    .bind(&id)
    .bind(next_connection_id)
    .bind(req.source_matcher.unwrap_or(current.source_matcher))
    .bind(req.source_name.unwrap_or(current.source_name))
    .bind(req.event_filter.unwrap_or(current.event_filter))
    .bind(req.trigger_config.unwrap_or(current.trigger_config))
    .bind(next_route_id)
    .bind(req.allow_direct_send.unwrap_or(current.allow_direct_send))
    .bind(
        req.close_task_on_close
            .unwrap_or(current.close_task_on_close),
    )
    .bind(req.notify_on_success.unwrap_or(current.notify_on_success))
    .bind(req.active.unwrap_or(current.active))
    .bind(next_scope)
    .bind(next_team_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(task_trigger_response(&row)))
}

#[utoipa::path(
    get,
    path = "/api/v1/triggers/{id}",
    tag = "triggers",
    params(("id" = String, Path, description = "Task trigger id")),
    responses(
        (status = 200, description = "Task trigger", body = TaskTriggerResponse),
        (status = 404, description = "Not found", body = crate::error::ErrorBody),
    )
)]
pub async fn get_task_trigger(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<TaskTriggerResponse>, ChannelError> {
    let row = load_task_trigger_for_auth(&state, &auth.scope_context(), &id).await?;
    Ok(Json(task_trigger_response(&row)))
}

#[utoipa::path(
    get,
    path = "/api/v1/triggers/{id}/tasks",
    tag = "triggers",
    params(
        ("id" = String, Path, description = "Task trigger id"),
        ("limit" = Option<u32>, Query, description = "Max items, default 10, cap 50"),
    ),
    responses((status = 200, body = Vec<TriggerRecentTaskItem>))
)]
pub async fn list_trigger_recent_tasks(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<RecentTasksQuery>,
) -> Result<Json<Vec<TriggerRecentTaskItem>>, ChannelError> {
    // Auth check: load_task_trigger_for_auth enforces scope on the trigger.
    // Tasks reached via task_bindings.trigger_id inherit the trigger's scope
    // by construction, so no additional task-level scope predicate is needed.
    let _ = load_task_trigger_for_auth(&state, &auth.scope_context(), &id).await?;
    let limit = query.limit.unwrap_or(10).min(50) as i64;
    let rows = PgTaskBindingRepository(state.db.clone())
        .find_recent_for_trigger(&id, limit)
        .await?;
    Ok(Json(rows.iter().map(map_recent_task_item).collect()))
}

#[utoipa::path(
    delete,
    path = "/api/v1/triggers/{id}",
    tag = "triggers",
    params(("id" = String, Path, description = "Task trigger id")),
    responses(
        (status = 204, description = "Task trigger deleted"),
        (status = 404, description = "Task trigger not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_task_trigger(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ChannelError> {
    let ctx = auth.scope_context();
    load_task_trigger_for_auth(&state, &ctx, &id).await?;
    let result = sqlx::query("DELETE FROM task_triggers WHERE id = $1")
        .bind(&id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ChannelError::NotFound(format!("task trigger {id}")));
    }

    Ok(StatusCode::NO_CONTENT)
}
