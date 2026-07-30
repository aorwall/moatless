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

use crate::adapters::common::{
    ensure_task_access, load_adapter_connection_for_auth, load_loop_for_auth, task_binding_response,
};
use crate::adapters::error::AdapterError;
use crate::adapters::models::TaskBinding;
use crate::bindings::reply::{ExternalReplyService, resolve_message_target};
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
    let rows = state.task_bindings.find_by_task_id(&task_id).await?;
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
        let trigger = load_loop_for_auth(&state, &ctx, trigger_id).await?;
        let crate::loops::dao::LoopSourceRow::AdapterEvent(source) = trigger.source else {
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
        external_status: None,
        external_state_kind: None,
        external_status_raw: None,
        external_review_decision: None,
        external_mergeable_state: None,
        external_status_updated_at: None,
        active: true,
        deactivated_at: None,
    };
    let saved = state.task_bindings.upsert(&row).await?;
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
    let response = ExternalReplyService::new(&state)
        .send_task_reply(
            &ctx,
            &task_id,
            req.binding_id.as_deref(),
            &req.text,
            &req.file_ids,
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
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn add_message_reaction(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(message_id): Path<String>,
    Json(req): Json<AddMessageReactionRequest>,
) -> Result<Json<MessageReactionResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (binding, ext_id, provider_key) = resolve_message_target(&state, &ctx, &message_id).await?;
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("No adapters configured".into()))?;
    let adapter = providers.reactions_by_name(&provider_key)?;
    let binding_ctx = crate::bindings::runtime::binding_context(&binding);
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
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn remove_message_reaction(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((message_id, emoji)): Path<(String, String)>,
) -> Result<Json<MessageReactionResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (binding, ext_id, provider_key) = resolve_message_target(&state, &ctx, &message_id).await?;
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("No adapters configured".into()))?;
    let adapter = providers.reactions_by_name(&provider_key)?;
    let binding_ctx = crate::bindings::runtime::binding_context(&binding);
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
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn reply_to_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(message_id): Path<String>,
    Json(req): Json<ReplyToMessageRequest>,
) -> Result<Json<ReplyToMessageResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let response = ExternalReplyService::new(&state)
        .reply_to_message(&ctx, &message_id, &req.text, &req.file_ids)
        .await?;
    Ok(Json(response))
}
