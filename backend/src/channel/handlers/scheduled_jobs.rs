use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::channel::{
    CreateScheduledJobRequest, ScheduledJobResponse, UpdateScheduledJobRequest,
};

use super::common::{
    default_scope, ensure_can_write_scope, ensure_repository_exists, load_scheduled_job_for_auth,
    load_task_route_for_auth, next_fire_at, scheduled_job_response,
};
use crate::channel::error::ChannelError;
use crate::channel::models::ScheduledJob;
use crate::extractors::AuthUser;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/v1/scheduled-jobs",
    tag = "scheduled-jobs",
    responses((status = 200, description = "Scheduled jobs", body = Vec<ScheduledJobResponse>))
)]
pub async fn list_scheduled_jobs(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ScheduledJobResponse>>, ChannelError> {
    let ctx = auth.scope_context();
    let rows = sqlx::query_as::<_, ScheduledJob>(
        "SELECT * FROM scheduled_jobs ORDER BY next_fire_at ASC, created_at DESC",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.iter()
            .filter(|row| ctx.can_access(row.scope, row.team_id.as_deref(), Some(&row.created_by)))
            .map(scheduled_job_response)
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/scheduled-jobs",
    tag = "scheduled-jobs",
    request_body = CreateScheduledJobRequest,
    responses((status = 200, description = "Scheduled job created", body = ScheduledJobResponse))
)]
pub async fn create_scheduled_job(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateScheduledJobRequest>,
) -> Result<Json<ScheduledJobResponse>, ChannelError> {
    let ctx = auth.scope_context();
    let route = load_task_route_for_auth(&state, &ctx, &req.route_id).await?;
    ensure_repository_exists(&state, &ctx, &route.repository_id).await?;
    let scope = default_scope(req.scope, req.team_id.as_deref());
    ensure_can_write_scope(&ctx, scope, req.team_id.as_deref())?;

    let now = Utc::now();
    let next = next_fire_at(&req.schedule_expr)?;
    let row = sqlx::query_as::<_, ScheduledJob>(
        r#"INSERT INTO scheduled_jobs
           (id, route_id, schedule_expr, timezone, message_template,
            run_as_user_id, active, next_fire_at, created_by, scope, team_id,
            created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
           RETURNING *"#,
    )
    .bind(format!("sj_{}", uuid::Uuid::new_v4()))
    .bind(req.route_id)
    .bind(req.schedule_expr)
    .bind(req.timezone.unwrap_or_else(|| "UTC".to_string()))
    .bind(req.message_template)
    .bind(req.run_as_user_id)
    .bind(req.active.unwrap_or(true))
    .bind(next)
    .bind(auth.user_id)
    .bind(scope)
    .bind(req.team_id)
    .bind(now)
    .bind(now)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(scheduled_job_response(&row)))
}

#[utoipa::path(
    put,
    path = "/api/v1/scheduled-jobs/{id}",
    tag = "scheduled-jobs",
    request_body = UpdateScheduledJobRequest,
    responses((status = 200, description = "Scheduled job updated", body = ScheduledJobResponse))
)]
pub async fn update_scheduled_job(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateScheduledJobRequest>,
) -> Result<Json<ScheduledJobResponse>, ChannelError> {
    let ctx = auth.scope_context();
    let current = load_scheduled_job_for_auth(&state, &ctx, &id).await?;
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

    let schedule_expr = req.schedule_expr.unwrap_or(current.schedule_expr);
    let next = next_fire_at(&schedule_expr)?;
    let row = sqlx::query_as::<_, ScheduledJob>(
        r#"UPDATE scheduled_jobs
           SET route_id = $2,
               schedule_expr = $3,
               timezone = $4,
               message_template = $5,
               run_as_user_id = $6,
               active = $7,
               next_fire_at = $8,
               scope = $9,
               team_id = $10,
               updated_at = now()
           WHERE id = $1
           RETURNING *"#,
    )
    .bind(&id)
    .bind(next_route_id)
    .bind(schedule_expr)
    .bind(req.timezone.unwrap_or(current.timezone))
    .bind(req.message_template.unwrap_or(current.message_template))
    .bind(req.run_as_user_id.unwrap_or(current.run_as_user_id))
    .bind(req.active.unwrap_or(current.active))
    .bind(next)
    .bind(next_scope)
    .bind(next_team_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(scheduled_job_response(&row)))
}
