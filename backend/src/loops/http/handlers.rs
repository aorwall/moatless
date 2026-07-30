use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::bindings::TaskBindingResponse;
use moatless_api_schemas::enums::TaskStatus;
use moatless_api_schemas::loops::{
    CreateLoopRequest, Loop, LoopKind, LoopRecentTaskItem, LoopRecentTaskSummary,
    UpdateLoopRequest, UpdateLoopSource,
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::adapters::common::{
    ensure_can_run_as, ensure_repository_exists, load_adapter_connection_for_auth,
    load_loop_for_auth, loop_response, next_fire_at, parse_adapter_kind,
    resolve_scope_for_adapter_create, LoopFilter,
};
use crate::adapters::error::AdapterError;
use crate::bindings::dao::LoopRecentTaskRow;
use crate::extractors::AuthUser;
use crate::loops::dao::{
    AdapterSourceUpdate, LoopListFilter, LoopSourceRow, LoopUpdate, NewAdapterEventSource, NewLoop,
    NewLoopConfig, NewLoopSource, NewScheduleSource, UpdateLoopSource as StoreUpdateSource,
};
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RecentTasksQuery {
    pub limit: Option<u32>,
}

fn map_recent_task_item(row: &LoopRecentTaskRow) -> LoopRecentTaskItem {
    let status = TaskStatus::try_from(row.status.as_str()).unwrap_or(TaskStatus::Open);
    let binding = match (
        row.binding_id.as_ref(),
        row.provider.as_ref(),
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
            adapter_kind: parse_adapter_kind(provider),
            connection_id: connection_id.clone(),
            trigger_id: row.trigger_id.clone(),
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
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<LoopFilter>,
) -> Result<Json<Vec<Loop>>, AdapterError> {
    let ctx = auth.scope_context();
    let kind = query.kind.map(|kind| kind.as_str().to_string());
    let adapter_kind = query
        .adapter_kind
        .map(|adapter_kind| adapter_kind.as_str().to_string());
    let records = state
        .loops
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
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateLoopRequest>,
) -> Result<Json<Loop>, AdapterError> {
    let ctx = auth.scope_context();
    ensure_repository_exists(&state, &ctx, &req.config.repository_id).await?;
    ensure_can_run_as(&state, &ctx, req.config.run_as_user_id.as_deref()).await?;

    let adapter_source = match (&req.kind, &req.source) {
        (
            LoopKind::AdapterEvent,
            moatless_api_schemas::loops::CreateLoopSource::AdapterEvent(source),
        ) => Some(source),
        _ => None,
    };
    let schedule_source = match (&req.kind, &req.source) {
        (LoopKind::Schedule, moatless_api_schemas::loops::CreateLoopSource::Schedule(source)) => {
            Some(source)
        }
        _ => None,
    };
    if matches!(req.kind, LoopKind::AdapterEvent) && adapter_source.is_none() {
        return Err(AdapterError::BadRequest(
            "adapter_event loops require an adapter-event source".into(),
        ));
    }
    if matches!(req.kind, LoopKind::Schedule) && schedule_source.is_none() {
        return Err(AdapterError::BadRequest(
            "schedule loops require a schedule source".into(),
        ));
    }

    if let Some(source) = adapter_source {
        let adapter_kind = source.adapter_kind.as_str().to_string();
        let connection =
            load_adapter_connection_for_auth(&state, &ctx, &source.connection_id).await?;
        if connection.provider != adapter_kind {
            return Err(AdapterError::BadRequest(format!(
                "Connection {} is for adapter {}, not {}",
                connection.id, connection.provider, adapter_kind
            )));
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
        let next_fire = next_fire
            .ok_or_else(|| AdapterError::Internal("missing schedule next fire".to_string()))?;
        NewLoopSource::Schedule(NewScheduleSource {
            cron_expression: source.cron_expression.clone(),
            timezone: source.timezone.clone().unwrap_or_else(|| "UTC".to_string()),
            message_template: source.message_template.clone(),
            next_fire_at: next_fire,
        })
    } else {
        NewLoopSource::Manual
    };

    state
        .loops
        .create(NewLoop {
            id: id.clone(),
            name: req.name.clone(),
            kind: req.kind.as_str().to_string(),
            active: req.active.unwrap_or(true),
            scope: resolved.scope,
            owner_user_id: resolved.owner_user_id,
            created_by: auth.user_id.to_string(),
            created_at: now,
            rule: NewLoopConfig {
                repository_id: req.config.repository_id.clone(),
                routing_mode: req.config.routing_mode,
                ongoing_task_id: req.config.ongoing_task_id.clone(),
                prompt: req.config.prompt.clone(),
                task_name: req.config.task_name.clone(),
                tag_ids: req.config.tag_ids.clone().unwrap_or_default(),
                run_as_user_id: req.config.run_as_user_id.clone(),
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

    let record = state
        .loops
        .get(&id, &ctx)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Loop {id} not found")))?;
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
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateLoopRequest>,
) -> Result<Json<Loop>, AdapterError> {
    let ctx = auth.scope_context();
    let current = load_loop_for_auth(&state, &ctx, &id).await?;

    if let Some(rule) = &req.config {
        if let Some(repository_id) = rule.repository_id.as_deref() {
            ensure_repository_exists(&state, &ctx, repository_id).await?;
        }
        if let Some(run_as_user_id) = rule.run_as_user_id.as_ref().and_then(|v| v.as_deref()) {
            ensure_can_run_as(&state, &ctx, Some(run_as_user_id)).await?;
        }
    }

    // Scope update: if a new scope is requested, resolve it; otherwise keep current.
    let (next_scope, next_owner) = if let Some(requested) = req.scope {
        let resolved = resolve_scope_for_adapter_create(&ctx, Some(requested))?;
        (resolved.scope, resolved.owner_user_id)
    } else {
        (
            current.trigger.scope,
            current.trigger.owner_user_id.clone(),
        )
    };

    if let Some(UpdateLoopSource::AdapterEvent(source)) = &req.source {
        let LoopSourceRow::AdapterEvent(current_source) = &current.source else {
            return Err(AdapterError::BadRequest(
                "adapter-event source updates are only valid for adapter_event loops".into(),
            ));
        };
        let next_connection_id = source
            .connection_id
            .clone()
            .unwrap_or_else(|| current_source.connection_id.clone());
        let connection =
            load_adapter_connection_for_auth(&state, &ctx, &next_connection_id).await?;
        if connection.provider != current_source.adapter_kind {
            return Err(AdapterError::BadRequest(format!(
                "Connection {} is for adapter {}, not {}",
                connection.id, connection.provider, current_source.adapter_kind
            )));
        }
    }
    let next_fire = if let Some(UpdateLoopSource::Schedule(source)) = &req.source {
        let LoopSourceRow::Schedule(current_source) = &current.source else {
            return Err(AdapterError::BadRequest(
                "schedule source updates are only valid for schedule loops".into(),
            ));
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
            .unwrap_or_else(|| current.rule.repository_id.clone()),
        routing_mode: rule.routing_mode.unwrap_or(current.rule.routing_mode),
        ongoing_task_id: rule
            .ongoing_task_id
            .unwrap_or_else(|| current.rule.ongoing_task_id.clone()),
        prompt: rule.prompt.unwrap_or_else(|| current.rule.prompt.clone()),
        task_name: rule
            .task_name
            .unwrap_or_else(|| current.rule.task_name.clone()),
        tag_ids: rule.tag_ids.unwrap_or_else(|| current.rule.tag_ids.clone()),
        run_as_user_id: rule
            .run_as_user_id
            .unwrap_or_else(|| current.rule.run_as_user_id.clone()),
        agent_type: rule
            .agent_type
            .unwrap_or_else(|| current.rule.agent_type.clone()),
        inactive_ttl_days: rule
            .inactive_ttl_days
            .unwrap_or(current.rule.inactive_ttl_days),
    });

    let source_update = match req.source {
        Some(UpdateLoopSource::AdapterEvent(source)) => {
            let LoopSourceRow::AdapterEvent(current_source) = &current.source else {
                return Err(AdapterError::BadRequest(
                    "adapter-event source updates are only valid for adapter_event loops".into(),
                ));
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
                return Err(AdapterError::BadRequest(
                    "schedule source updates are only valid for schedule loops".into(),
                ));
            };
            let next_fire = next_fire
                .ok_or_else(|| AdapterError::Internal("missing schedule next fire".to_string()))?;
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

    let updated = state
        .loops
        .update(
            &id,
            LoopUpdate {
                name: req.name.unwrap_or_else(|| current.trigger.name.clone()),
                active: req.active.unwrap_or(current.trigger.active),
                scope: next_scope,
                owner_user_id: next_owner,
            },
            rule_update,
            source_update,
            &ctx,
        )
        .await?;

    if !updated {
        return Err(AdapterError::NotFound(format!("trigger {id}")));
    }

    let record = state
        .loops
        .get(&id, &ctx)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Loop {id} not found")))?;
    Ok(Json(loop_response(&record)))
}

#[utoipa::path(
    get,
    path = "/api/v1/loops/{id}",
    tag = "loops",
    params(("id" = String, Path, description = "Loop id")),
    responses(
        (status = 200, description = "Loop", body = Loop),
        (status = 404, description = "Not found", body = crate::error::ErrorBody),
    )
)]
pub async fn get_loop(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Loop>, AdapterError> {
    let record = load_loop_for_auth(&state, &auth.scope_context(), &id).await?;
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
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<RecentTasksQuery>,
) -> Result<Json<Vec<LoopRecentTaskItem>>, AdapterError> {
    let _ = load_loop_for_auth(&state, &auth.scope_context(), &id).await?;
    let limit = query.limit.unwrap_or(10).min(50) as i64;
    let rows = state.task_bindings.find_recent_for_loop(&id, limit).await?;
    Ok(Json(rows.iter().map(map_recent_task_item).collect()))
}

#[utoipa::path(
    delete,
    path = "/api/v1/loops/{id}",
    tag = "loops",
    params(("id" = String, Path, description = "Loop id")),
    responses(
        (status = 204, description = "Loop deleted"),
        (status = 404, description = "Loop not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_loop(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AdapterError> {
    let ctx = auth.scope_context();
    let result = state.loops.delete(&id, &ctx).await?;

    if result == 0 {
        return Err(AdapterError::NotFound(format!("trigger {id}")));
    }

    Ok(StatusCode::NO_CONTENT)
}
