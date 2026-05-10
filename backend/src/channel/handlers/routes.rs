use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::channel::{
    CreateTaskRouteRequest, TaskRouteResponse, UpdateTaskRouteRequest,
};

use super::common::{
    ensure_can_run_as, ensure_can_write_channel_resource, ensure_repository_exists,
    load_task_route_for_auth, resolve_scope_for_channel_create, resolve_scope_for_channel_update,
    scoped_resource, task_route_response, RouteFilter, TeamIdUpdate,
};
use crate::channel::dao::routes::PgTaskRouteRepository;
use crate::channel::error::ChannelError;
use crate::channel::models::TaskRoute;
use crate::extractors::AuthUser;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/v1/task-routes",
    tag = "task-routes",
    responses((status = 200, description = "Task routes", body = Vec<TaskRouteResponse>))
)]
pub async fn list_task_routes(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<RouteFilter>,
) -> Result<Json<Vec<TaskRouteResponse>>, ChannelError> {
    let ctx = auth.scope_context();
    let rows = PgTaskRouteRepository(state.db.clone())
        .list(&ctx, query.repository_id.as_deref())
        .await?;
    Ok(Json(rows.iter().map(task_route_response).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/task-routes",
    tag = "task-routes",
    request_body = CreateTaskRouteRequest,
    responses((status = 200, description = "Task route created", body = TaskRouteResponse))
)]
pub async fn create_task_route(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateTaskRouteRequest>,
) -> Result<Json<TaskRouteResponse>, ChannelError> {
    let ctx = auth.scope_context();
    ensure_repository_exists(&state, &ctx, &req.repository_id).await?;
    ensure_can_run_as(&ctx, req.run_as_user_id.as_deref())?;
    let resolved =
        resolve_scope_for_channel_create(&state, &auth, req.scope, req.team_id.clone()).await?;

    let now = Utc::now();
    let row = sqlx::query_as::<_, TaskRoute>(
        r#"INSERT INTO task_routes
           (id, repository_id, routing_mode, ongoing_task_id, prompt_template,
            task_name, tag_ids, run_as_user_id, agent_type, scope, team_id,
            owner_user_id, created_by, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
           RETURNING *"#,
    )
    .bind(format!("route_{}", uuid::Uuid::new_v4()))
    .bind(req.repository_id)
    .bind(req.routing_mode)
    .bind(req.ongoing_task_id)
    .bind(req.prompt_template)
    .bind(req.task_name)
    .bind(req.tag_ids.unwrap_or_default())
    .bind(req.run_as_user_id)
    .bind(req.agent_type.unwrap_or_else(|| "claude-code".to_string()))
    .bind(resolved.scope)
    .bind(Some(resolved.team_id))
    .bind(resolved.owner_user_id)
    .bind(auth.user_id)
    .bind(now)
    .bind(now)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(task_route_response(&row)))
}

#[utoipa::path(
    put,
    path = "/api/v1/task-routes/{id}",
    tag = "task-routes",
    request_body = UpdateTaskRouteRequest,
    responses((status = 200, description = "Task route updated", body = TaskRouteResponse))
)]
pub async fn update_task_route(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateTaskRouteRequest>,
) -> Result<Json<TaskRouteResponse>, ChannelError> {
    let ctx = auth.scope_context();
    let current = load_task_route_for_auth(&state, &ctx, &id).await?;
    let current_resource = scoped_resource(
        current.scope,
        current.team_id.as_deref(),
        current.owner_user_id.as_deref(),
        &current.created_by,
    );
    ensure_can_write_channel_resource(&state, &auth, &current_resource, "task route").await?;

    if let Some(repository_id) = req.repository_id.as_deref() {
        ensure_repository_exists(&state, &ctx, repository_id).await?;
    }
    if let Some(run_as_user_id) = req
        .run_as_user_id
        .as_ref()
        .and_then(|value| value.as_deref())
    {
        ensure_can_run_as(&ctx, Some(run_as_user_id))?;
    }
    let resolved = resolve_scope_for_channel_update(
        &state,
        &auth,
        req.scope,
        req.team_id
            .clone()
            .map_or(TeamIdUpdate::Unchanged, TeamIdUpdate::Set),
        &current_resource,
    )
    .await?;

    let row = sqlx::query_as::<_, TaskRoute>(
        r#"UPDATE task_routes
           SET repository_id = $2,
               routing_mode = $3,
               ongoing_task_id = $4,
               prompt_template = $5,
               task_name = $6,
               tag_ids = $7,
               run_as_user_id = $8,
               agent_type = $9,
               scope = $10,
               team_id = $11,
               owner_user_id = $12,
               updated_at = now()
           WHERE id = $1
           RETURNING *"#,
    )
    .bind(&id)
    .bind(req.repository_id.unwrap_or(current.repository_id))
    .bind(req.routing_mode.unwrap_or(current.routing_mode))
    .bind(req.ongoing_task_id.unwrap_or(current.ongoing_task_id))
    .bind(req.prompt_template.unwrap_or(current.prompt_template))
    .bind(req.task_name.unwrap_or(current.task_name))
    .bind(req.tag_ids.unwrap_or(current.tag_ids))
    .bind(req.run_as_user_id.unwrap_or(current.run_as_user_id))
    .bind(req.agent_type.unwrap_or(current.agent_type))
    .bind(resolved.scope)
    .bind(Some(resolved.team_id))
    .bind(resolved.owner_user_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(task_route_response(&row)))
}
