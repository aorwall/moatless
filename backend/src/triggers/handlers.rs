use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::bindings::TaskBindingResponse;
use moatless_api_schemas::enums::TaskStatus;
use moatless_api_schemas::triggers::{
    CreateAdapterEventTriggerSource, CreateScheduleTriggerSource, CreateTriggerRequest,
    TriggerKind, TriggerRecentTaskItem, TriggerRecentTaskSummary, TriggerResponse,
    UpdateAdapterEventTriggerSource, UpdateScheduleTriggerSource, UpdateTriggerRequest,
    UpdateTriggerSource,
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::adapters::common::{
    ensure_can_run_as, ensure_can_write_adapter_resource, ensure_repository_exists,
    load_adapter_connection_for_auth, load_trigger_for_auth, next_fire_at, parse_adapter_kind,
    resolve_scope_for_adapter_create, resolve_scope_for_adapter_update, scoped_resource,
    trigger_response, TeamIdUpdate, TriggerFilter,
};
use crate::adapters::error::AdapterError;
use crate::bindings::dao::{PgTaskBindingRepository, TriggerRecentTaskRow};
use crate::extractors::AuthUser;
use crate::state::AppState;
use crate::triggers::dao::{PgTriggerRepository, TriggerListFilter, TriggerSourceRow};

#[derive(Debug, Deserialize, ToSchema)]
pub struct RecentTasksQuery {
    pub limit: Option<u32>,
}

fn map_recent_task_item(row: &TriggerRecentTaskRow) -> TriggerRecentTaskItem {
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
    TriggerRecentTaskItem {
        task: TriggerRecentTaskSummary {
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
    path = "/api/v1/triggers",
    tag = "triggers",
    responses((status = 200, description = "Triggers", body = Vec<TriggerResponse>))
)]
pub async fn list_triggers(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<TriggerFilter>,
) -> Result<Json<Vec<TriggerResponse>>, AdapterError> {
    let ctx = auth.scope_context();
    let kind = query.kind.map(|kind| kind.as_str().to_string());
    let adapter_kind = query
        .adapter_kind
        .map(|adapter_kind| adapter_kind.as_str().to_string());
    let records = PgTriggerRepository(state.db.clone())
        .list(
            &ctx,
            TriggerListFilter {
                kind: kind.as_deref(),
                adapter_kind: adapter_kind.as_deref(),
            },
        )
        .await?;
    Ok(Json(records.iter().map(trigger_response).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/triggers",
    tag = "triggers",
    request_body = CreateTriggerRequest,
    responses((status = 200, description = "Trigger created", body = TriggerResponse))
)]
pub async fn create_trigger(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateTriggerRequest>,
) -> Result<Json<TriggerResponse>, AdapterError> {
    let ctx = auth.scope_context();
    ensure_repository_exists(&state, &ctx, &req.rule.repository_id).await?;
    ensure_can_run_as(&ctx, req.rule.run_as_user_id.as_deref())?;

    let adapter_source = match (&req.kind, &req.source) {
        (
            TriggerKind::AdapterEvent,
            moatless_api_schemas::triggers::CreateTriggerSource::AdapterEvent(source),
        ) => Some(source),
        _ => None,
    };
    let schedule_source = match (&req.kind, &req.source) {
        (
            TriggerKind::Schedule,
            moatless_api_schemas::triggers::CreateTriggerSource::Schedule(source),
        ) => Some(source),
        _ => None,
    };
    if matches!(req.kind, TriggerKind::AdapterEvent) && adapter_source.is_none() {
        return Err(AdapterError::BadRequest(
            "adapter_event triggers require an adapter-event source".into(),
        ));
    }
    if matches!(req.kind, TriggerKind::Schedule) && schedule_source.is_none() {
        return Err(AdapterError::BadRequest(
            "schedule triggers require a schedule source".into(),
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

    let resolved =
        resolve_scope_for_adapter_create(&state, &auth, req.scope, req.team_id.clone()).await?;
    let id = format!("tr_{}", uuid::Uuid::new_v4());
    let now = Utc::now();
    let mut tx = state.db.begin().await?;

    sqlx::query(
        r#"INSERT INTO triggers
           (id, name, kind, active, scope, team_id, owner_user_id,
            created_by, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)"#,
    )
    .bind(&id)
    .bind(&req.name)
    .bind(req.kind.as_str())
    .bind(req.active.unwrap_or(true))
    .bind(resolved.scope)
    .bind(Some(resolved.team_id))
    .bind(resolved.owner_user_id)
    .bind(&auth.user_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO trigger_rules
           (trigger_id, repository_id, routing_mode, ongoing_task_id, prompt,
            task_name, tag_ids, run_as_user_id, agent_type, inactive_ttl_days)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(&id)
    .bind(&req.rule.repository_id)
    .bind(req.rule.routing_mode)
    .bind(&req.rule.ongoing_task_id)
    .bind(&req.rule.prompt)
    .bind(&req.rule.task_name)
    .bind(req.rule.tag_ids.unwrap_or_default())
    .bind(&req.rule.run_as_user_id)
    .bind(
        req.rule
            .agent_type
            .unwrap_or_else(|| "claude-code".to_string()),
    )
    .bind(req.rule.inactive_ttl_days)
    .execute(&mut *tx)
    .await?;

    if let Some(source) = adapter_source {
        insert_adapter_event_source(&mut tx, &id, source).await?;
    }
    if let Some(source) = schedule_source {
        let next_fire = next_fire
            .ok_or_else(|| AdapterError::Internal("missing schedule next fire".to_string()))?;
        insert_schedule_source(&mut tx, &id, source, next_fire).await?;
    }
    tx.commit().await?;

    let record = PgTriggerRepository(state.db.clone())
        .find_by_id(&id)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Trigger {id} not found")))?;
    Ok(Json(trigger_response(&record)))
}

async fn insert_adapter_event_source(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    trigger_id: &str,
    source: &CreateAdapterEventTriggerSource,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO adapter_event_trigger_sources
           (trigger_id, adapter_kind, connection_id, source_matcher, source_name,
            event_filter, adapter_config, allow_direct_send, close_task_on_close,
            notify_on_success)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(trigger_id)
    .bind(source.adapter_kind.as_str())
    .bind(&source.connection_id)
    .bind(&source.source_matcher)
    .bind(&source.source_name)
    .bind(
        source
            .event_filter
            .clone()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .bind(&source.adapter_config)
    .bind(source.allow_direct_send.unwrap_or(false))
    .bind(source.close_task_on_close.unwrap_or(true))
    .bind(source.notify_on_success.unwrap_or(false))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_schedule_source(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    trigger_id: &str,
    source: &CreateScheduleTriggerSource,
    next_fire: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO schedule_trigger_sources
           (trigger_id, cron_expression, timezone, message_template, next_fire_at)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(trigger_id)
    .bind(&source.cron_expression)
    .bind(source.timezone.as_deref().unwrap_or("UTC"))
    .bind(&source.message_template)
    .bind(next_fire)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[utoipa::path(
    put,
    path = "/api/v1/triggers/{id}",
    tag = "triggers",
    request_body = UpdateTriggerRequest,
    responses((status = 200, description = "Trigger updated", body = TriggerResponse))
)]
pub async fn update_trigger(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateTriggerRequest>,
) -> Result<Json<TriggerResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let current = load_trigger_for_auth(&state, &ctx, &id).await?;
    let current_resource = scoped_resource(
        current.trigger.scope,
        current.trigger.team_id.as_deref(),
        current.trigger.owner_user_id.as_deref(),
        &current.trigger.created_by,
    );
    ensure_can_write_adapter_resource(&state, &auth, &current_resource, "trigger").await?;

    if let Some(rule) = &req.rule {
        if let Some(repository_id) = rule.repository_id.as_deref() {
            ensure_repository_exists(&state, &ctx, repository_id).await?;
        }
        if let Some(run_as_user_id) = rule.run_as_user_id.as_ref().and_then(|v| v.as_deref()) {
            ensure_can_run_as(&ctx, Some(run_as_user_id))?;
        }
    }

    let resolved = resolve_scope_for_adapter_update(
        &state,
        &auth,
        req.scope,
        req.team_id
            .clone()
            .map_or(TeamIdUpdate::Unchanged, TeamIdUpdate::Set),
        &current_resource,
    )
    .await?;

    if let Some(UpdateTriggerSource::AdapterEvent(source)) = &req.source {
        let TriggerSourceRow::AdapterEvent(current_source) = &current.source else {
            return Err(AdapterError::BadRequest(
                "adapter-event source updates are only valid for adapter_event triggers".into(),
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
    let next_fire = if let Some(UpdateTriggerSource::Schedule(source)) = &req.source {
        let TriggerSourceRow::Schedule(current_source) = &current.source else {
            return Err(AdapterError::BadRequest(
                "schedule source updates are only valid for schedule triggers".into(),
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

    let mut tx = state.db.begin().await?;
    sqlx::query(
        r#"UPDATE triggers
           SET name = $2,
               active = $3,
               scope = $4,
               team_id = $5,
               owner_user_id = $6,
               updated_at = now()
           WHERE id = $1"#,
    )
    .bind(&id)
    .bind(req.name.unwrap_or_else(|| current.trigger.name.clone()))
    .bind(req.active.unwrap_or(current.trigger.active))
    .bind(resolved.scope)
    .bind(Some(resolved.team_id))
    .bind(resolved.owner_user_id)
    .execute(&mut *tx)
    .await?;

    if let Some(rule) = req.rule {
        sqlx::query(
            r#"UPDATE trigger_rules
               SET repository_id = $2,
                   routing_mode = $3,
                   ongoing_task_id = $4,
                   prompt = $5,
                   task_name = $6,
                   tag_ids = $7,
                   run_as_user_id = $8,
                   agent_type = $9,
                   inactive_ttl_days = $10
               WHERE trigger_id = $1"#,
        )
        .bind(&id)
        .bind(
            rule.repository_id
                .unwrap_or_else(|| current.rule.repository_id.clone()),
        )
        .bind(rule.routing_mode.unwrap_or(current.rule.routing_mode))
        .bind(
            rule.ongoing_task_id
                .unwrap_or_else(|| current.rule.ongoing_task_id.clone()),
        )
        .bind(rule.prompt.unwrap_or_else(|| current.rule.prompt.clone()))
        .bind(
            rule.task_name
                .unwrap_or_else(|| current.rule.task_name.clone()),
        )
        .bind(rule.tag_ids.unwrap_or_else(|| current.rule.tag_ids.clone()))
        .bind(
            rule.run_as_user_id
                .unwrap_or_else(|| current.rule.run_as_user_id.clone()),
        )
        .bind(
            rule.agent_type
                .unwrap_or_else(|| current.rule.agent_type.clone()),
        )
        .bind(
            rule.inactive_ttl_days
                .unwrap_or(current.rule.inactive_ttl_days),
        )
        .execute(&mut *tx)
        .await?;
    }

    if let Some(source) = req.source {
        match source {
            UpdateTriggerSource::AdapterEvent(source) => {
                update_adapter_event_source(&mut tx, &id, &current.source, source).await?;
            }
            UpdateTriggerSource::Schedule(source) => {
                let next_fire = next_fire.ok_or_else(|| {
                    AdapterError::Internal("missing schedule next fire".to_string())
                })?;
                update_schedule_source(&mut tx, &id, &current.source, source, next_fire).await?;
            }
            UpdateTriggerSource::Manual(_) => {}
        }
    }
    tx.commit().await?;

    let record = PgTriggerRepository(state.db.clone())
        .find_by_id(&id)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Trigger {id} not found")))?;
    Ok(Json(trigger_response(&record)))
}

async fn update_adapter_event_source(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    trigger_id: &str,
    current: &TriggerSourceRow,
    source: UpdateAdapterEventTriggerSource,
) -> Result<(), AdapterError> {
    let TriggerSourceRow::AdapterEvent(current_source) = current else {
        return Err(AdapterError::BadRequest(
            "adapter-event source updates are only valid for adapter_event triggers".into(),
        ));
    };
    sqlx::query(
        r#"UPDATE adapter_event_trigger_sources
           SET connection_id = $2,
               source_matcher = $3,
               source_name = $4,
               event_filter = $5,
               adapter_config = $6,
               allow_direct_send = $7,
               close_task_on_close = $8,
               notify_on_success = $9
           WHERE trigger_id = $1"#,
    )
    .bind(trigger_id)
    .bind(
        source
            .connection_id
            .unwrap_or_else(|| current_source.connection_id.clone()),
    )
    .bind(
        source
            .source_matcher
            .unwrap_or_else(|| current_source.source_matcher.clone()),
    )
    .bind(
        source
            .source_name
            .unwrap_or_else(|| current_source.source_name.clone()),
    )
    .bind(
        source
            .event_filter
            .unwrap_or_else(|| current_source.event_filter.clone()),
    )
    .bind(
        source
            .adapter_config
            .unwrap_or_else(|| current_source.adapter_config.clone()),
    )
    .bind(
        source
            .allow_direct_send
            .unwrap_or(current_source.allow_direct_send),
    )
    .bind(
        source
            .close_task_on_close
            .unwrap_or(current_source.close_task_on_close),
    )
    .bind(
        source
            .notify_on_success
            .unwrap_or(current_source.notify_on_success),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn update_schedule_source(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    trigger_id: &str,
    current: &TriggerSourceRow,
    source: UpdateScheduleTriggerSource,
    next_fire: chrono::DateTime<Utc>,
) -> Result<(), AdapterError> {
    let TriggerSourceRow::Schedule(current_source) = current else {
        return Err(AdapterError::BadRequest(
            "schedule source updates are only valid for schedule triggers".into(),
        ));
    };
    sqlx::query(
        r#"UPDATE schedule_trigger_sources
           SET cron_expression = $2,
               timezone = $3,
               message_template = $4,
               next_fire_at = $5
           WHERE trigger_id = $1"#,
    )
    .bind(trigger_id)
    .bind(
        source
            .cron_expression
            .unwrap_or_else(|| current_source.cron_expression.clone()),
    )
    .bind(
        source
            .timezone
            .unwrap_or_else(|| current_source.timezone.clone()),
    )
    .bind(
        source
            .message_template
            .unwrap_or_else(|| current_source.message_template.clone()),
    )
    .bind(next_fire)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/v1/triggers/{id}",
    tag = "triggers",
    params(("id" = String, Path, description = "Trigger id")),
    responses(
        (status = 200, description = "Trigger", body = TriggerResponse),
        (status = 404, description = "Not found", body = crate::error::ErrorBody),
    )
)]
pub async fn get_trigger(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<TriggerResponse>, AdapterError> {
    let record = load_trigger_for_auth(&state, &auth.scope_context(), &id).await?;
    Ok(Json(trigger_response(&record)))
}

#[utoipa::path(
    get,
    path = "/api/v1/triggers/{id}/tasks",
    tag = "triggers",
    params(
        ("id" = String, Path, description = "Trigger id"),
        ("limit" = Option<u32>, Query, description = "Max items, default 10, cap 50"),
    ),
    responses((status = 200, body = Vec<TriggerRecentTaskItem>))
)]
pub async fn list_trigger_recent_tasks(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<RecentTasksQuery>,
) -> Result<Json<Vec<TriggerRecentTaskItem>>, AdapterError> {
    let _ = load_trigger_for_auth(&state, &auth.scope_context(), &id).await?;
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
    params(("id" = String, Path, description = "Trigger id")),
    responses(
        (status = 204, description = "Trigger deleted"),
        (status = 404, description = "Trigger not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_trigger(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AdapterError> {
    let ctx = auth.scope_context();
    let current = load_trigger_for_auth(&state, &ctx, &id).await?;
    let resource = scoped_resource(
        current.trigger.scope,
        current.trigger.team_id.as_deref(),
        current.trigger.owner_user_id.as_deref(),
        &current.trigger.created_by,
    );
    ensure_can_write_adapter_resource(&state, &auth, &resource, "trigger").await?;
    let result = PgTriggerRepository(state.db.clone()).delete(&id).await?;

    if result == 0 {
        return Err(AdapterError::NotFound(format!("trigger {id}")));
    }

    Ok(StatusCode::NO_CONTENT)
}
