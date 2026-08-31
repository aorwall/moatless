use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use chrono::Utc;
use moatless_api_schemas::bindings::{
    CreateTaskBindingRequest, SendTaskReplyRequest, SendTaskReplyResponse, TaskBindingResponse,
};
use moatless_api_schemas::message::{
    AddMessageReactionRequest, MessageReactionResponse, ReplyToMessageRequest,
    ReplyToMessageResponse,
};

use moatless_adapter_model::registry::Adapters;
use crate::adapters::common::{
    ensure_task_access_only, ensure_task_access_with, load_adapter_connection_for_auth_with,
    load_loop_for_auth_with,
    task_binding_response,
};
use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use crate::bindings::dao::TaskBindingDao;
use crate::bindings::reply::ExternalReplyService;
use moatless_core::http_auth::AuthUser;
use crate::loops::dao::LoopDao;
use moatless_backend_core::task::TaskReader;
use moatless_adapter_model::models::TaskBinding;

#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/bindings",
    tag = "task-bindings",
    responses((status = 200, description = "Task bindings", body = Vec<TaskBindingResponse>))
)]
pub async fn list_task_bindings(
    State(tasks): State<Arc<dyn TaskReader>>,
    State(task_bindings): State<Arc<dyn TaskBindingDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<Vec<TaskBindingResponse>>, AdapterError> {
    let ctx = auth.scope_context();
    ensure_task_access_only(tasks.as_ref(), &ctx, &task_id).await?;
    let rows = task_bindings.find_by_task_id(&task_id).await?;
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
    State(tasks): State<Arc<dyn TaskReader>>,
    State(task_bindings): State<Arc<dyn TaskBindingDao>>,
    State(adapter_connections): State<Arc<dyn AdapterConnectionDao>>,
    State(loops): State<Arc<LoopDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<CreateTaskBindingRequest>,
) -> Result<Json<TaskBindingResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let task = ensure_task_access_with(tasks.as_ref(), &ctx, &task_id).await?;
    let adapter_kind = req.adapter_kind;
    let connection = load_adapter_connection_for_auth_with(
        adapter_connections.as_ref(),
        &ctx,
        &req.connection_id,
    )
    .await?;
    if connection.adapter_kind != adapter_kind {
        return Err(SharedAdapterError::BadRequest(format!(
            "Connection {} is for adapter {}, not {}",
            connection.id, connection.adapter_kind, adapter_kind
        ))
        .into());
    }
    if let Some(loop_id) = req.loop_id.as_deref() {
        let trigger = load_loop_for_auth_with(loops.as_ref(), &ctx, loop_id).await?;
        let crate::loops::dao::LoopSourceRow::AdapterEvent(source) = trigger.source else {
            return Err(SharedAdapterError::BadRequest(format!(
                "Loop {loop_id} is not an adapter-event Loop"
            ))
            .into());
        };
        if source.adapter_kind != adapter_kind {
            return Err(SharedAdapterError::BadRequest(format!(
                "Loop {} is for adapter {}, not {}",
                trigger.loop_.id, source.adapter_kind, adapter_kind
            ))
            .into());
        }
        if source.connection_id != req.connection_id {
            return Err(SharedAdapterError::BadRequest(format!(
                "Loop {} belongs to connection {}, not {}",
                trigger.loop_.id, source.connection_id, req.connection_id
            ))
            .into());
        }
    }
    let now = Utc::now();
    let row = TaskBinding {
        id: format!("tb_{}", uuid::Uuid::new_v4()),
        task_id: task.task_id,
        adapter_kind,
        subject_kind: req.subject_kind,
        subject_key: req.subject_key,
        connection_id: req.connection_id,
        loop_id: req.loop_id,
        destination_config: req
            .destination_config
            .unwrap_or_else(|| serde_json::json!({})),
        metadata: req.metadata.unwrap_or_else(|| serde_json::json!({})),
        last_external_event_id: None,
        origin: req.origin.unwrap_or_else(|| "api".to_string()),
        created_at: now,
        last_event_at: now,
        external_status: None,
        external_state_kind: None,
        external_status_raw: None,
        external_review_decision: None,
        external_mergeable_state: None,
        external_status_updated_at: None,
        active: true,
        deactivated_at: None,
    };
    let saved = task_bindings.upsert(&row).await?;
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
    State(tasks): State<Arc<dyn TaskReader>>,
    State(external_reply): State<Arc<ExternalReplyService>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SendTaskReplyRequest>,
) -> Result<Json<SendTaskReplyResponse>, AdapterError> {
    let ctx = auth.scope_context();
    ensure_task_access_only(tasks.as_ref(), &ctx, &task_id).await?;
    let response = external_reply
        .send_task_reply(
            &ctx,
            &task_id,
            req.binding_id.as_deref(),
            &req.text,
            &req.file_ids,
            req.presentation,
        )
        .await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/messages/{message_id}/reactions",
    tag = "task-bindings",
    request_body = AddMessageReactionRequest,
    responses(
        (status = 200, description = "Reaction added", body = MessageReactionResponse),
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn add_message_reaction(
    State(external_reply): State<Arc<ExternalReplyService>>,
    State(adapters): State<Option<Arc<Adapters>>>,
    auth: AuthUser,
    Path(message_id): Path<String>,
    Json(req): Json<AddMessageReactionRequest>,
) -> Result<Json<MessageReactionResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (binding, ext_id, provider_key) = external_reply
        .resolve_message_target(&ctx, &message_id)
        .await?;
    let providers = adapters
        .as_ref()
        .ok_or_else(|| SharedAdapterError::AdapterNotConfigured("No adapters configured".into()))?;
    let adapter = providers.reactions_by_name(&provider_key)?;
    let binding_ctx = moatless_adapter_model::models::binding_context(&binding);
    let external_id = adapter
        .add_reaction(&binding_ctx, &ext_id, &req.emoji)
        .await?;
    Ok(Json(MessageReactionResponse {
        ok: true,
        external_id,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/v1/messages/{message_id}/reactions/{emoji}",
    tag = "task-bindings",
    responses(
        (status = 200, description = "Reaction removed", body = MessageReactionResponse),
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn remove_message_reaction(
    State(external_reply): State<Arc<ExternalReplyService>>,
    State(adapters): State<Option<Arc<Adapters>>>,
    auth: AuthUser,
    Path((message_id, emoji)): Path<(String, String)>,
) -> Result<Json<MessageReactionResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (binding, ext_id, provider_key) = external_reply
        .resolve_message_target(&ctx, &message_id)
        .await?;
    let providers = adapters
        .as_ref()
        .ok_or_else(|| SharedAdapterError::AdapterNotConfigured("No adapters configured".into()))?;
    let adapter = providers.reactions_by_name(&provider_key)?;
    let binding_ctx = moatless_adapter_model::models::binding_context(&binding);
    adapter
        .remove_reaction(&binding_ctx, &ext_id, &emoji)
        .await?;
    Ok(Json(MessageReactionResponse {
        ok: true,
        external_id: None,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/messages/{message_id}/reply",
    tag = "task-bindings",
    request_body = ReplyToMessageRequest,
    responses(
        (status = 200, description = "Reply sent", body = ReplyToMessageResponse),
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn reply_to_message(
    State(external_reply): State<Arc<ExternalReplyService>>,
    auth: AuthUser,
    Path(message_id): Path<String>,
    Json(req): Json<ReplyToMessageRequest>,
) -> Result<Json<ReplyToMessageResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let response = external_reply
        .reply_to_message(&ctx, &message_id, &req.text, &req.file_ids, req.presentation)
        .await?;
    Ok(Json(response))
}
