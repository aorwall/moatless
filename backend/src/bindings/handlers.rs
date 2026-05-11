use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::bindings::{
    CreateTaskBindingRequest, SendTaskReplyRequest, SendTaskReplyResponse, TaskBindingResponse,
};

use crate::adapters::common::{
    ensure_task_access, load_adapter_connection_for_auth, load_trigger_for_auth,
    task_binding_response,
};
use crate::adapters::error::AdapterError;
use crate::adapters::models::TaskBinding;
use crate::bindings::dao::PgTaskBindingRepository;
use crate::bindings::reply::ReplyService;
use crate::extractors::AuthUser;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/bindings",
    tag = "task-bindings",
    responses((status = 200, description = "Task bindings", body = Vec<TaskBindingResponse>))
)]
pub async fn list_task_bindings(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<Vec<TaskBindingResponse>>, AdapterError> {
    let ctx = auth.scope_context();
    ensure_task_access(&state, &ctx, &task_id).await?;
    let rows = PgTaskBindingRepository(state.db.clone())
        .find_by_task_id(&task_id)
        .await?;
    Ok(Json(rows.iter().map(task_binding_response).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/bindings",
    tag = "task-bindings",
    request_body = CreateTaskBindingRequest,
    responses((status = 200, description = "Task binding created", body = TaskBindingResponse))
)]
pub async fn create_task_binding(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<CreateTaskBindingRequest>,
) -> Result<Json<TaskBindingResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let task = ensure_task_access(&state, &ctx, &task_id).await?;
    let provider = req.adapter_kind.as_str().to_string();
    let connection = load_adapter_connection_for_auth(&state, &ctx, &req.connection_id).await?;
    if connection.provider != provider {
        return Err(AdapterError::BadRequest(format!(
            "Connection {} is for provider {}, not {}",
            connection.id, connection.provider, provider
        )));
    }
    if let Some(trigger_id) = req.trigger_id.as_deref() {
        let trigger = load_trigger_for_auth(&state, &ctx, trigger_id).await?;
        let crate::triggers::dao::TriggerSourceRow::AdapterEvent(source) = trigger.source else {
            return Err(AdapterError::BadRequest(format!(
                "Trigger {trigger_id} is not an adapter-event trigger"
            )));
        };
        if source.adapter_kind != provider {
            return Err(AdapterError::BadRequest(format!(
                "Trigger {} is for adapter {}, not {}",
                trigger.trigger.id, source.adapter_kind, provider
            )));
        }
        if source.connection_id != req.connection_id {
            return Err(AdapterError::BadRequest(format!(
                "Trigger {} belongs to connection {}, not {}",
                trigger.trigger.id, source.connection_id, req.connection_id
            )));
        }
    }
    let now = Utc::now();
    let row = TaskBinding {
        id: format!("tb_{}", uuid::Uuid::new_v4()),
        task_id: task.task_id,
        provider,
        subject_kind: req.subject_kind,
        subject_key: req.subject_key,
        connection_id: req.connection_id,
        trigger_id: req.trigger_id,
        destination_config: req
            .destination_config
            .unwrap_or_else(|| serde_json::json!({})),
        metadata: req.metadata.unwrap_or_else(|| serde_json::json!({})),
        last_external_event_id: None,
        origin: req.origin.unwrap_or_else(|| "api".to_string()),
        created_at: now,
        last_event_at: now,
    };
    let saved = PgTaskBindingRepository(state.db.clone())
        .upsert(&row)
        .await?;
    Ok(Json(task_binding_response(&saved)))
}

#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/reply",
    tag = "task-bindings",
    request_body = SendTaskReplyRequest,
    responses((status = 200, description = "Reply sent", body = SendTaskReplyResponse))
)]
pub async fn send_task_reply(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SendTaskReplyRequest>,
) -> Result<Json<SendTaskReplyResponse>, AdapterError> {
    let ctx = auth.scope_context();
    ensure_task_access(&state, &ctx, &task_id).await?;
    let response = ReplyService::new(&state)
        .send_reply(
            &ctx,
            &task_id,
            req.binding_id.as_deref(),
            &req.text,
            &req.file_ids,
        )
        .await?;
    Ok(Json(response))
}
