use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use moatless_api_schemas::bindings::TaskBindingResponse;
use moatless_api_schemas::enums::TaskStatus;
use moatless_api_schemas::loops::{ActivateLoopRequest, CreateLoopRequest, Loop, LoopRecentTaskItem, LoopRecentTaskSummary, ResetGitLoopResponse, UpdateLoopRequest, UpdateLoopSource};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::adapters::common::{LoopFilter, ensure_repository_exists, load_adapter_connection_for_auth_with, load_loop_for_auth_with, loop_response, next_fire_at, resolve_scope_for_adapter_create};
use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_adapter_model::models::parse_adapter_kind;
use crate::adapters::error::{AdapterError, SharedAdapterError};
use crate::loops::dao::{AdapterSourceUpdate, LoopDao, LoopListFilter, LoopSourceRow, LoopUpdate, NewAdapterEventSource, NewLoop, NewLoopConfig, NewLoopSource, NewScheduleSource, UpdateLoopSource as StoreUpdateSource};
use moatless_types::loops::LoopKind as DomainLoopKind;
use crate::loops::sync::LoopSyncService;
use moatless_backend_core::identity::UserResolver;
use moatless_core::http_auth::AuthUser;
use crate::bindings::dao::{LoopRecentTaskRow, TaskBindingDao};
use moatless_backend_core::workspace::RepositoryReader;

fn require_admin(auth: &AuthUser) -> Result<(), AdapterError> {
    if auth.role.as_deref() == Some("admin") {
        Ok(())
    } else {
        Err(
            SharedAdapterError::AccessDenied("Only administrators can modify Loops".to_string())
                .into(),
        )
    }
}

/// Every Loop carries a name, so whitespace is not one.
fn require_name(name: &str) -> Result<String, AdapterError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SharedAdapterError::BadRequest("A Loop needs a name".to_string()).into());
    }
    Ok(trimmed.to_string())
}

async fn resolve_run_as_user(
    users: &dyn UserResolver,
    requested: &str,
) -> Result<String, AdapterError> {
    let canonical = users
        .canonical_id(requested)
        .await
        .map_err(|e| SharedAdapterError::Internal(e.to_string()))?;
    canonical.map(|id| id.to_string()).ok_or_else(|| {
        SharedAdapterError::BadRequest(format!("Run-as user {requested} does not exist")).into()
    })
}

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
            adapter_kind: (parse_adapter_kind(provider)),
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
    require_admin(&auth)?;
    let ctx = auth.scope_context();
    let name = require_name(&req.name)?;
    ensure_repository_exists(repositories.as_ref(), &ctx, &req.config.repository_id).await?;
    let run_as_user_id = match req.config.run_as_user_id.as_deref() {
        Some(requested) => Some(resolve_run_as_user(users.as_ref(), requested).await?),
        None => None,
    };
    let active = req.active.unwrap_or(false);
    if active && run_as_user_id.is_none() {
        return Err(SharedAdapterError::BadRequest(
            "Activating a Loop requires an explicit run-as user".to_string(),
        )
        .into());
    }

    let kind = req.kind;
    let adapter_source = match (kind, &req.source) {
        (
            DomainLoopKind::AdapterEvent,
            moatless_api_schemas::loops::CreateLoopSource::AdapterEvent(source),
        ) => Some(source),
        _ => None,
    };
    let schedule_source = match (kind, &req.source) {
        (
            DomainLoopKind::Schedule,
            moatless_api_schemas::loops::CreateLoopSource::Schedule(source),
        ) => Some(source),
        _ => None,
    };
    if matches!(kind, DomainLoopKind::AdapterEvent) && adapter_source.is_none() {
        return Err(SharedAdapterError::BadRequest(
            "adapter_event loops require an adapter-event source".into(),
        )
        .into());
    }
    if matches!(kind, DomainLoopKind::Schedule) && schedule_source.is_none() {
        return Err(SharedAdapterError::BadRequest(
            "schedule loops require a schedule source".into(),
        )
        .into());
    }

    if let Some(source) = adapter_source {
        let adapter_kind = source.adapter_kind.as_str().to_string();
        let connection = load_adapter_connection_for_auth_with(
            adapter_connections.as_ref(),
            &ctx,
            &source.connection_id,
        )
        .await?;
        if connection.adapter_kind.as_str() != adapter_kind {
            return Err(SharedAdapterError::BadRequest(format!(
                "Connection {} is for adapter {}, not {}",
                connection.id, connection.adapter_kind, adapter_kind
            ))
            .into());
        }
    }

    let next_fire = if let Some(source) = schedule_source {
        Some(next_fire_at(&source.cron_expression)?)
    } else {
        None
    };

    let resolved = resolve_scope_for_adapter_create(&ctx, req.scope)?;
    let id = format!("tr_{}", uuid::Uuid::new_v4());
    let now = Utc::now();

    let source = if let Some(source) = adapter_source {
        NewLoopSource::AdapterEvent(NewAdapterEventSource {
            adapter_kind: source.adapter_kind.as_str().to_string(),
            connection_id: source.connection_id.clone(),
            source_matcher: source.source_matcher.clone(),
            source_name: source.source_name.clone(),
            event_filter: source
                .event_filter
                .clone()
                .unwrap_or_else(|| serde_json::json!([])),
            adapter_config: source.adapter_config.clone(),
            allow_direct_send: source.allow_direct_send.unwrap_or(false),
            close_task_on_close: source.close_task_on_close.unwrap_or(true),
            notify_on_success: source.notify_on_success.unwrap_or(false),
        })
    } else if let Some(source) = schedule_source {
        let next_fire = next_fire.ok_or_else(|| {
            SharedAdapterError::Internal("missing schedule next fire".to_string())
        })?;
        NewLoopSource::Schedule(NewScheduleSource {
            cron_expression: source.cron_expression.clone(),
            timezone: source.timezone.clone().unwrap_or_else(|| "UTC".to_string()),
            message_template: source.message_template.clone(),
            next_fire_at: next_fire,
        })
    } else {
        NewLoopSource::Manual
    };

    loops
        .create(NewLoop {
            id: id.clone(),
            name,
            kind: kind.as_str().to_string(),
            active,
            scope: resolved.scope,
            owner_user_id: resolved.owner_user_id,
            created_by: auth.user_id.to_string(),
            created_at: now,
            config: NewLoopConfig {
                repository_id: req.config.repository_id.clone(),
                routing_mode: req.config.routing_mode,
                ongoing_task_id: req.config.ongoing_task_id.clone(),
                prompt: req.config.prompt.clone(),
                task_name: req.config.task_name.clone(),
                tag_ids: req.config.tag_ids.clone().unwrap_or_default(),
                skills: req.config.skills.clone().unwrap_or_default(),
                run_as_user_id,
                agent_type: req
                    .config
                    .agent_type
                    .clone()
                    .unwrap_or_else(|| "claude-code".to_string()),
                inactive_ttl_days: req.config.inactive_ttl_days,
            },
            source,
        })
        .await?;

    let record = loops
        .get(&id, &ctx)
        .await?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")))?;
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
    require_admin(&auth)?;
    let ctx = auth.scope_context();
    let current = load_loop_for_auth_with(loops.as_ref(), &ctx, &id).await?;
    if current.loop_.config_source != moatless_types::loops::LoopConfigSource::Manual || current.loop_.deleted {
        return Err(SharedAdapterError::Conflict(
            "Git-synced Loops are read-only; override the Loop before editing it".to_string(),
        )
        .into());
    }

    let next_name = match req.name.as_deref() {
        Some(name) => require_name(name)?,
        None => current.loop_.name.clone(),
    };

    if let Some(rule) = &req.config {
        if let Some(repository_id) = rule.repository_id.as_deref() {
            ensure_repository_exists(repositories.as_ref(), &ctx, repository_id).await?;
        }
    }

    // Scope update: if a new scope is requested, resolve it; otherwise keep current.
    let (next_scope, next_owner) = if let Some(requested) = req.scope {
        let resolved = resolve_scope_for_adapter_create(&ctx, Some(requested))?;
        (resolved.scope, resolved.owner_user_id)
    } else {
        (current.loop_.scope, current.loop_.owner_user_id.clone())
    };

    if let Some(UpdateLoopSource::AdapterEvent(source)) = &req.source {
        let LoopSourceRow::AdapterEvent(current_source) = &current.source else {
            return Err(SharedAdapterError::BadRequest(
                "adapter-event source updates are only valid for adapter_event loops".into(),
            )
            .into());
        };
        let next_connection_id = source
            .connection_id
            .clone()
            .unwrap_or_else(|| current_source.connection_id.clone());
        let connection = load_adapter_connection_for_auth_with(
            adapter_connections.as_ref(),
            &ctx,
            &next_connection_id,
        )
        .await?;
        if connection.adapter_kind != current_source.adapter_kind {
            return Err(SharedAdapterError::BadRequest(format!(
                "Connection {} is for adapter {}, not {}",
                connection.id, connection.adapter_kind, current_source.adapter_kind
            ))
            .into());
        }
    }
    let next_fire = if let Some(UpdateLoopSource::Schedule(source)) = &req.source {
        let LoopSourceRow::Schedule(current_source) = &current.source else {
            return Err(SharedAdapterError::BadRequest(
                "schedule source updates are only valid for schedule loops".into(),
            )
            .into());
        };
        let expr = source
            .cron_expression
            .as_deref()
            .unwrap_or(&current_source.cron_expression);
        Some(next_fire_at(expr)?)
    } else {
        None
    };

    let rule_update = req.config.map(|rule| NewLoopConfig {
        repository_id: rule
            .repository_id
            .unwrap_or_else(|| current.config.repository_id.clone()),
        routing_mode: rule
            .routing_mode
            .unwrap_or(current.config.routing_mode),
        ongoing_task_id: rule
            .ongoing_task_id
            .unwrap_or_else(|| current.config.ongoing_task_id.clone()),
        prompt: rule.prompt.unwrap_or_else(|| current.config.prompt.clone()),
        task_name: rule
            .task_name
            .unwrap_or_else(|| current.config.task_name.clone()),
        tag_ids: rule
            .tag_ids
            .unwrap_or_else(|| current.config.tag_ids.clone()),
        skills: rule.skills.unwrap_or_else(|| current.config.skills.clone()),
        run_as_user_id: current.config.run_as_user_id.clone(),
        agent_type: rule
            .agent_type
            .unwrap_or_else(|| current.config.agent_type.as_str().to_string()),
        inactive_ttl_days: rule
            .inactive_ttl_days
            .unwrap_or(current.config.inactive_ttl_days),
    });

    let source_update = match req.source {
        Some(UpdateLoopSource::AdapterEvent(source)) => {
            let LoopSourceRow::AdapterEvent(current_source) = &current.source else {
                return Err(SharedAdapterError::BadRequest(
                    "adapter-event source updates are only valid for adapter_event loops".into(),
                )
                .into());
            };
            Some(StoreUpdateSource::AdapterEvent(AdapterSourceUpdate {
                connection_id: source
                    .connection_id
                    .unwrap_or_else(|| current_source.connection_id.clone()),
                source_matcher: source
                    .source_matcher
                    .unwrap_or_else(|| current_source.source_matcher.clone()),
                source_name: source
                    .source_name
                    .unwrap_or_else(|| current_source.source_name.clone()),
                event_filter: source
                    .event_filter
                    .unwrap_or_else(|| current_source.event_filter.clone()),
                adapter_config: source
                    .adapter_config
                    .unwrap_or_else(|| current_source.adapter_config.clone()),
                allow_direct_send: source
                    .allow_direct_send
                    .unwrap_or(current_source.allow_direct_send),
                close_task_on_close: source
                    .close_task_on_close
                    .unwrap_or(current_source.close_task_on_close),
                notify_on_success: source
                    .notify_on_success
                    .unwrap_or(current_source.notify_on_success),
            }))
        }
        Some(UpdateLoopSource::Schedule(source)) => {
            let LoopSourceRow::Schedule(current_source) = &current.source else {
                return Err(SharedAdapterError::BadRequest(
                    "schedule source updates are only valid for schedule loops".into(),
                )
                .into());
            };
            let next_fire = next_fire.ok_or_else(|| {
                SharedAdapterError::Internal("missing schedule next fire".to_string())
            })?;
            Some(StoreUpdateSource::Schedule(NewScheduleSource {
                cron_expression: source
                    .cron_expression
                    .unwrap_or_else(|| current_source.cron_expression.clone()),
                timezone: source
                    .timezone
                    .unwrap_or_else(|| current_source.timezone.clone()),
                message_template: source
                    .message_template
                    .unwrap_or_else(|| current_source.message_template.clone()),
                next_fire_at: next_fire,
            }))
        }
        Some(UpdateLoopSource::Manual(_)) | None => None,
    };

    let updated = loops
        .update(
            &id,
            LoopUpdate {
                name: next_name,
                scope: next_scope,
                owner_user_id: next_owner,
            },
            rule_update,
            source_update,
            &ctx,
        )
        .await?;

    if !updated {
        return Err(SharedAdapterError::NotFound(format!("Loop {id}")).into());
    }

    let record = loops
        .get(&id, &ctx)
        .await?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")))?;
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
    require_admin(&auth)?;
    let ctx = auth.scope_context();
    let result = loops.delete(&id, &ctx).await?;

    if result == 0 {
        return Err(SharedAdapterError::NotFound(format!("Loop {id}")).into());
    }

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
    require_admin(&auth)?;
    let ctx = auth.scope_context();
    let run_as_user_id = resolve_run_as_user(users.as_ref(), &req.run_as_user_id).await?;
    if !loops.activate(&id, &run_as_user_id, &ctx).await? {
        return Err(SharedAdapterError::NotFound(format!("Loop {id} not found")).into());
    }
    updated_loop(loops.as_ref(), &ctx, &id).await
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
    require_admin(&auth)?;
    let ctx = auth.scope_context();
    if !loops.pause(&id, &ctx).await? {
        return Err(SharedAdapterError::NotFound(format!("Loop {id} not found")).into());
    }
    updated_loop(loops.as_ref(), &ctx, &id).await
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
    require_admin(&auth)?;
    let ctx = auth.scope_context();
    if !loops.resume(&id, &ctx).await? {
        return Err(SharedAdapterError::Conflict(
            "Loop cannot resume until an administrator selects a run-as user".to_string(),
        )
        .into());
    }
    updated_loop(loops.as_ref(), &ctx, &id).await
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
    require_admin(&auth)?;
    let ctx = auth.scope_context();
    if !loops.override_git(&id, &ctx).await? {
        return Err(SharedAdapterError::Conflict(
            "Only a non-deleted Git-synced Loop can be overridden".to_string(),
        )
        .into());
    }
    updated_loop(loops.as_ref(), &ctx, &id).await
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
    require_admin(&auth)?;
    let ctx = auth.scope_context();
    let current = loops
        .get(&id, &ctx)
        .await?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")))?;
    let repository_id = current
        .loop_
        .source_repository_id
        .as_deref()
        .ok_or_else(|| SharedAdapterError::BadRequest("Loop has no Git provenance".to_string()))?;
    let source_key =
        current.loop_.source_key.as_deref().ok_or_else(|| {
            SharedAdapterError::BadRequest("Loop has no Git source key".to_string())
        })?;
    let repository = repositories
        .describe(&ctx, repository_id)
        .await?
        .ok_or_else(|| {
            SharedAdapterError::NotFound(format!("Repository {repository_id} not found"))
        })?;

    let result = loop_sync
        .reset_repo_loop(&repository, &auth, source_key)
        .await;
    let record = loops
        .get(&id, &ctx)
        .await?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")))?;
    let (resynced, error) = match result {
        Ok(true) => (true, None),
        Ok(false) => (false, Some("Loop provenance no longer exists".to_string())),
        Err(error) => (false, Some(error.to_string())),
    };
    Ok(Json(ResetGitLoopResponse {
        loop_: loop_response(&record),
        resynced,
        error,
    }))
}
