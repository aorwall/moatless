use super::projection::{AdapterFilter, adapter_connection_response};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_api_schemas::adapters::{
    AdapterConnectionResponse, AdapterSendRequest, AdapterSendResponse,
    CreateAdapterConnectionRequest, OpenTaskRoomRequest, OpenTaskRoomResponse,
};
use moatless_api_schemas::user::{SendDmRequest, SendDmResponse};
use moatless_core::http_auth::AuthUser;
use moatless_types::adapter::AdapterKind;

#[utoipa::path(
    get,
    path = "/api/v1/adapters",
    tag = "adapters",
    responses((status = 200, description = "Configured adapter names", body = Vec<String>))
)]
pub async fn list_adapters(State(state): State<AppState>) -> Json<Vec<String>> {
    let types = state
        .adapters
        .as_ref()
        .map_or_default(|p| p.registered_types());

    let adapters = types
        .into_iter()
        .map(|t| match t {
            AdapterKind::Slack => "slack".to_string(),
            AdapterKind::Github => "github".to_string(),
            AdapterKind::GithubPr => "github".to_string(),
            AdapterKind::Linear => "linear".to_string(),
            AdapterKind::Webhook => "webhook".to_string(),
        })
        .collect();

    Json(adapters)
}

#[utoipa::path(
    get,
    path = "/api/v1/adapters/connections",
    tag = "adapters",
    responses((status = 200, description = "Adapter connections", body = Vec<AdapterConnectionResponse>))
)]
pub async fn list_adapter_connections(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<AdapterFilter>,
) -> Result<Json<Vec<AdapterConnectionResponse>>, AdapterError> {
    let ctx = auth.scope_context();
    let rows = state
        .adapter_connections
        .list(&ctx, query.adapter_kind)
        .await?;
    Ok(Json(rows.iter().map(adapter_connection_response).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/adapters/connections",
    tag = "adapters",
    request_body = CreateAdapterConnectionRequest,
    responses((status = 200, description = "Adapter connection upserted", body = AdapterConnectionResponse))
)]
pub async fn create_adapter_connection(
    State(commands): State<std::sync::Arc<super::operations::AdapterCommands>>,
    auth: AuthUser,
    Json(req): Json<CreateAdapterConnectionRequest>,
) -> Result<Json<AdapterConnectionResponse>, AdapterError> {
    commands
        .create_adapter_connection(&auth.scope_context(), req)
        .await
        .map(Json)
}

#[utoipa::path(
    delete,
    path = "/api/v1/adapters/connections/{id}",
    tag = "adapters",
    params(("id" = String, Path, description = "Adapter connection id")),
    responses(
        (status = 204, description = "Adapter connection deleted"),
        (status = 404, description = "Connection not found", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn delete_adapter_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, AdapterError> {
    let ctx = auth.scope_context();
    // The store applies the control filter, so a connection the caller
    // cannot control deletes nothing and reads back as not found — the same
    // answer they would get for one that never existed.
    if state.adapter_connections.delete(&ctx, &id).await? == 0 {
        return Err(
            SharedAdapterError::NotFound(format!("Adapter connection not found: {id}")).into(),
        );
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/adapters/send",
    tag = "adapters",
    request_body = AdapterSendRequest,
    responses(
        (status = 200, description = "Adapter send result", body = AdapterSendResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Connection or task not found", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn adapter_send(
    State(commands): State<std::sync::Arc<super::operations::AdapterCommands>>,
    auth: AuthUser,
    Json(req): Json<AdapterSendRequest>,
) -> Result<Json<AdapterSendResponse>, AdapterError> {
    commands
        .adapter_send(&auth.scope_context(), req)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/room",
    tag = "adapters",
    params(("task_id" = String, Path, description = "Task ID")),
    request_body = OpenTaskRoomRequest,
    responses(
        (status = 200, description = "Room requested", body = OpenTaskRoomResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task or connection not found", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn open_task_room(
    State(commands): State<std::sync::Arc<super::operations::AdapterCommands>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<OpenTaskRoomRequest>,
) -> Result<Json<OpenTaskRoomResponse>, AdapterError> {
    commands
        .open_task_room(&auth.scope_context(), &task_id, req)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/v1/users/{login}/dm",
    tag = "users",
    params(("login" = String, Path, description = "Moatless user login")),
    request_body = SendDmRequest,
    responses(
        (status = 200, description = "DM sent", body = SendDmResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "User or provider mapping not found", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn send_user_dm(
    State(commands): State<std::sync::Arc<super::operations::AdapterCommands>>,
    auth: AuthUser,
    Path(login): Path<String>,
    Json(req): Json<SendDmRequest>,
) -> Result<Json<SendDmResponse>, AdapterError> {
    commands
        .send_user_dm(&auth.scope_context(), login, req)
        .await
        .map(Json)
}
