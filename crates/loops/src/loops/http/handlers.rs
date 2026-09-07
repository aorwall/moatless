use std::sync::Arc;

use moatless_types::adapter::AdapterKind;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use moatless_api_schemas::bindings::TaskBindingResponse;
use moatless_api_schemas::enums::TaskStatus;
use moatless_api_schemas::loops::{
    ActivateLoopRequest, CreateLoopRequest, Loop, LoopRecentTaskItem, LoopRecentTaskSummary,
    ResetGitLoopResponse, UpdateLoopRequest,
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::bindings::dao::{LoopRecentTaskRow, TaskBindingDao};
use crate::loops::access::load_loop_for_auth_with;
use crate::loops::dao::{LoopDao, LoopListFilter};
use crate::loops::http::projection::LoopFilter;
use crate::loops::http::projection::loop_response;
use crate::loops::sync::LoopSyncService;
use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_core::http_auth::AuthUser;
use moatless_ports::identity::UserResolver;
use moatless_ports::workspace::RepositoryReader;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RecentTasksQuery {
    pub limit: Option<u32>,
}

fn map_recent_task_item(row: &LoopRecentTaskRow) -> LoopRecentTaskItem {
    let status = TaskStatus::try_from(row.status.as_str()).unwrap_or(TaskStatus::Open);
    let binding = match (
        row.binding_id.as_ref(),
        row.adapter_kind.as_ref(),
        row.connection_id.as_ref(),
        row.subject_kind.as_ref(),
        row.subject_key.as_ref(),
        row.destination_config.as_ref(),
        row.metadata.as_ref(),
        row.origin.as_ref(),
        row.binding_created_at,
        row.last_event_at,
    ) {
        (
            Some(id),
            Some(provider),
            Some(connection_id),
            Some(subject_kind),
            Some(subject_key),
            Some(destination_config),
            Some(metadata),
            Some(origin),
            Some(binding_created_at),
            Some(last_event_at),
        ) => Some(TaskBindingResponse {
            id: id.clone(),
            task_id: row.task_id.clone(),
            adapter_kind: (AdapterKind::parse_or_webhook(provider)),
            connection_id: connection_id.clone(),
            loop_id: row.loop_id.clone(),
            subject_kind: subject_kind.clone(),
            subject_key: subject_key.clone(),
            destination_config: destination_config.clone(),
            metadata: metadata.clone(),
            last_external_event_id: row.last_external_event_id.clone(),
            origin: origin.clone(),
            created_at: binding_created_at.to_rfc3339(),
            last_event_at: last_event_at.to_rfc3339(),
        }),
        _ => None,
    };
    LoopRecentTaskItem {
        task: LoopRecentTaskSummary {
            task_id: row.task_id.clone(),
            short_id: row.short_id.clone(),
            title: row.title.clone().unwrap_or_default(),
            status,
            created_at: row.task_created_at.clone(),
        },
        binding,
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/loops",
    tag = "loops",
    responses((status = 200, description = "Loops", body = Vec<Loop>))
)]
pub async fn list_loops(
    State(loops): State<Arc<LoopDao>>,
    auth: AuthUser,
    Query(query): Query<LoopFilter>,
) -> Result<Json<Vec<Loop>>, AdapterError> {
    let ctx = auth.scope_context();
    let kind = query.kind.map(|kind| kind.as_str().to_string());
    let adapter_kind = query
        .adapter_kind
        .map(|adapter_kind| adapter_kind.as_str().to_string());
    let records = loops
        .list(
            &ctx,
            LoopListFilter {
                kind: kind.as_deref(),
                adapter_kind: adapter_kind.as_deref(),
            },
        )
        .await?;
    Ok(Json(records.iter().map(loop_response).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/loops",
    tag = "loops",
    request_body = CreateLoopRequest,
    responses((status = 200, description = "Loop created", body = Loop))
)]
pub async fn create_loop(
    State(loops): State<Arc<LoopDao>>,
    State(users): State<Arc<dyn UserResolver>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    State(adapter_connections): State<Arc<dyn AdapterConnectionDao>>,
    auth: AuthUser,
    Json(req): Json<CreateLoopRequest>,
) -> Result<Json<Loop>, AdapterError> {
    let record = super::super::mutations::create_loop(
        loops.as_ref(),
        users.as_ref(),
        repositories.as_ref(),
        adapter_connections.as_ref(),
        &auth.scope_context(),
        req,
    )
    .await?;
    Ok(Json(loop_response(&record)))
}

#[utoipa::path(
    put,
    path = "/api/v1/loops/{id}",
    tag = "loops",
    request_body = UpdateLoopRequest,
    responses((status = 200, description = "Loop updated", body = Loop))
)]
pub async fn update_loop(
    State(loops): State<Arc<LoopDao>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    State(adapter_connections): State<Arc<dyn AdapterConnectionDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateLoopRequest>,
) -> Result<Json<Loop>, AdapterError> {
    let record = super::super::mutations::update_loop(
        loops.as_ref(),
        repositories.as_ref(),
        adapter_connections.as_ref(),
        &auth.scope_context(),
        &id,
        req,
    )
    .await?;
    Ok(Json(loop_response(&record)))
}

#[utoipa::path(
    get,
    path = "/api/v1/loops/{id}",
    tag = "loops",
    params(("id" = String, Path, description = "Loop id")),
    responses(
        (status = 200, description = "Loop", body = Loop),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn get_loop(
    State(loops): State<Arc<LoopDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Loop>, AdapterError> {
    let record = load_loop_for_auth_with(loops.as_ref(), &auth.scope_context(), &id).await?;
    Ok(Json(loop_response(&record)))
}

#[utoipa::path(
    get,
    path = "/api/v1/loops/{id}/tasks",
    tag = "loops",
    params(
        ("id" = String, Path, description = "Loop id"),
        ("limit" = Option<u32>, Query, description = "Max items, default 10, cap 50"),
    ),
    responses((status = 200, body = Vec<LoopRecentTaskItem>))
)]
pub async fn list_loop_recent_tasks(
    State(loops): State<Arc<LoopDao>>,
    State(task_bindings): State<Arc<dyn TaskBindingDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<RecentTasksQuery>,
) -> Result<Json<Vec<LoopRecentTaskItem>>, AdapterError> {
    let _ = load_loop_for_auth_with(loops.as_ref(), &auth.scope_context(), &id).await?;
    let limit = query.limit.unwrap_or(10).min(50) as i64;
    let rows = task_bindings.find_recent_for_loop(&id, limit).await?;
    Ok(Json(rows.iter().map(map_recent_task_item).collect()))
}

#[utoipa::path(
    delete,
    path = "/api/v1/loops/{id}",
    tag = "loops",
    params(("id" = String, Path, description = "Loop id")),
    responses(
        (status = 204, description = "Loop deleted"),
        (status = 404, description = "Loop not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_loop(
    State(loops): State<Arc<LoopDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AdapterError> {
    super::super::mutations::delete_loop(loops.as_ref(), &auth.scope_context(), &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn updated_loop(
    loops: &LoopDao,
    ctx: &moatless_types::access_control::ScopeContext,
    id: &str,
) -> Result<Json<Loop>, AdapterError> {
    let record = loops
        .get(id, ctx)
        .await?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")))?;
    Ok(Json(loop_response(&record)))
}

#[utoipa::path(
    post,
    path = "/api/v1/loops/{id}/activate",
    tag = "loops",
    request_body = ActivateLoopRequest,
    responses((status = 200, description = "Loop activated", body = Loop))
)]
pub async fn activate_loop(
    State(loops): State<Arc<LoopDao>>,
    State(users): State<Arc<dyn UserResolver>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<ActivateLoopRequest>,
) -> Result<Json<Loop>, AdapterError> {
    super::super::mutations::activate_loop(
        loops.as_ref(),
        users.as_ref(),
        &auth.scope_context(),
        &id,
        &req.run_as_user_id,
    )
    .await?;
    updated_loop(loops.as_ref(), &auth.scope_context(), &id).await
}

#[utoipa::path(
    post,
    path = "/api/v1/loops/{id}/pause",
    tag = "loops",
    responses((status = 200, description = "Loop paused", body = Loop))
)]
pub async fn pause_loop(
    State(loops): State<Arc<LoopDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Loop>, AdapterError> {
    super::super::mutations::pause_loop(loops.as_ref(), &auth.scope_context(), &id).await?;
    updated_loop(loops.as_ref(), &auth.scope_context(), &id).await
}

#[utoipa::path(
    post,
    path = "/api/v1/loops/{id}/resume",
    tag = "loops",
    responses((status = 200, description = "Loop resumed", body = Loop))
)]
pub async fn resume_loop(
    State(loops): State<Arc<LoopDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Loop>, AdapterError> {
    super::super::mutations::resume_loop(loops.as_ref(), &auth.scope_context(), &id).await?;
    updated_loop(loops.as_ref(), &auth.scope_context(), &id).await
}

#[utoipa::path(
    post,
    path = "/api/v1/loops/{id}/override",
    tag = "loops",
    responses((status = 200, description = "Git Loop overridden", body = Loop))
)]
pub async fn override_loop(
    State(loops): State<Arc<LoopDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Loop>, AdapterError> {
    super::super::mutations::override_loop(loops.as_ref(), &auth.scope_context(), &id).await?;
    updated_loop(loops.as_ref(), &auth.scope_context(), &id).await
}

#[utoipa::path(
    post,
    path = "/api/v1/loops/{id}/reset-git",
    tag = "loops",
    responses((status = 200, description = "Loop reset result", body = ResetGitLoopResponse))
)]
pub async fn reset_git_loop(
    State(loops): State<Arc<LoopDao>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    State(loop_sync): State<Arc<LoopSyncService>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ResetGitLoopResponse>, AdapterError> {
    let outcome = super::super::mutations::reset_git_loop(
        loops.as_ref(),
        repositories.as_ref(),
        loop_sync.as_ref(),
        &auth.scope_context(),
        &id,
    )
    .await?;
    Ok(Json(ResetGitLoopResponse {
        loop_: loop_response(&outcome.record),
        resynced: outcome.resynced,
        error: outcome.error,
    }))
}
