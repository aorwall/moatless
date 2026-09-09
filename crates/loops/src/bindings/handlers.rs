use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use moatless_api_schemas::bindings::{
    CreateTaskBindingRequest, SendTaskReplyRequest, SendTaskReplyResponse, TaskBindingResponse,
};
use moatless_api_schemas::message::{
    AddMessageReactionRequest, MessageReactionResponse, ReplyToMessageRequest,
    ReplyToMessageResponse,
};

use crate::bindings::access::ensure_task_access_only;
use crate::bindings::operations::Bindings;
use crate::bindings::projection::task_binding_response;
use crate::bindings::reply::ExternalReplyService;
use crate::loops::operations::Loops;
use moatless_adapter_model::connections::AdapterConnections;
use moatless_adapter_model::error::AdapterError;
use moatless_core::http_auth::AuthUser;
use moatless_ports::task::TaskReader;

#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/bindings",
    tag = "task-bindings",
    responses((status = 200, description = "Task bindings", body = Vec<TaskBindingResponse>))
)]
pub async fn list_task_bindings(
    State(tasks): State<Arc<dyn TaskReader>>,
    State(task_bindings): State<Arc<Bindings>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<Vec<TaskBindingResponse>>, AdapterError> {
    let ctx = auth.scope_context();
    ensure_task_access_only(tasks.as_ref(), &ctx, &task_id).await?;
    let rows = task_bindings.for_task(&task_id).await?;
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
    State(task_bindings): State<Arc<Bindings>>,
    State(adapter_connections): State<Arc<AdapterConnections>>,
    State(loops): State<Arc<Loops>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<CreateTaskBindingRequest>,
) -> Result<Json<TaskBindingResponse>, AdapterError> {
    let saved = super::creation::create_task_binding(
        tasks.as_ref(),
        task_bindings.as_ref(),
        adapter_connections.as_ref(),
        loops.as_ref(),
        &auth.scope_context(),
        &task_id,
        req,
    )
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
    auth: AuthUser,
    Path(message_id): Path<String>,
    Json(req): Json<AddMessageReactionRequest>,
) -> Result<Json<MessageReactionResponse>, AdapterError> {
    Ok(Json(
        external_reply
            .add_message_reaction(&auth.scope_context(), &message_id, &req.emoji)
            .await?,
    ))
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
    auth: AuthUser,
    Path((message_id, emoji)): Path<(String, String)>,
) -> Result<Json<MessageReactionResponse>, AdapterError> {
    Ok(Json(
        external_reply
            .remove_message_reaction(&auth.scope_context(), &message_id, &emoji)
            .await?,
    ))
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
        .reply_to_message(
            &ctx,
            &message_id,
            &req.text,
            &req.file_ids,
            req.presentation,
        )
        .await?;
    Ok(Json(response))
}
