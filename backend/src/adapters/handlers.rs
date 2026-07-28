use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::adapters::{
    AdapterConnectionResponse, AdapterSendRequest, AdapterSendResponse,
    CreateAdapterConnectionRequest,
};
use moatless_api_schemas::user::{SendDmRequest, SendDmResponse};

use super::common::{
    adapter_connection_response, ensure_task_access, resolve_connection,
    resolve_scope_for_adapter_create, AdapterFilter,
};
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, OutboundReply};
use crate::adapters::AdapterKind;
use crate::bindings::outbound::{resolve_files, truncate_text};
use crate::bindings::runtime::{upsert_binding, ORIGIN_CONNECTED_THREAD};
use crate::extractors::AuthUser;
use crate::state::AppState;

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
        .map(|p| p.registered_types())
        .unwrap_or_default();

    let adapters = types
        .into_iter()
        .map(|t| match t {
            AdapterKind::Slack => "slack".to_string(),
            AdapterKind::Telegram => "telegram".to_string(),
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
    let provider = query.adapter_kind.map(|p| p.as_str().to_string());
    let rows = state
        .adapter_connections
        .list(&ctx, provider.as_deref())
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
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateAdapterConnectionRequest>,
) -> Result<Json<AdapterConnectionResponse>, AdapterError> {
    if req.external_account_id.trim().is_empty() {
        return Err(AdapterError::BadRequest(
            "external_account_id is required".into(),
        ));
    }

    let ctx = auth.scope_context();
    let now = Utc::now();
    let provider = req.adapter_kind.as_str().to_string();
    let resolved = resolve_scope_for_adapter_create(&ctx, req.scope)?;
    let repo = &state.adapter_connections;
    let row = AdapterConnection {
        id: format!("pc_{}", uuid::Uuid::new_v4()),
        provider,
        connection_kind: req.connection_kind.unwrap_or_else(|| "webhook".to_string()),
        scope_value: req.external_account_id,
        credential_ref: None,
        webhook_secret: req.webhook_secret,
        transport_config: req
            .transport_config
            .unwrap_or_else(|| serde_json::json!({})),
        created_by: auth.user_id.to_string(),
        scope: resolved.scope,
        owner_user_id: resolved.owner_user_id,
        created_at: now,
        updated_at: now,
    };
    let saved = repo.upsert(&row, &ctx).await?;
    Ok(Json(adapter_connection_response(&saved)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/adapters/connections/{id}",
    tag = "adapters",
    params(("id" = String, Path, description = "Adapter connection id")),
    responses(
        (status = 204, description = "Adapter connection deleted"),
        (status = 404, description = "Connection not found", body = crate::error::ErrorBody),
    )
)]
pub async fn delete_adapter_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, AdapterError> {
    let ctx = auth.scope_context();
    // The repository applies the control filter, so a connection the caller
    // cannot control deletes nothing and reads back as not found — the same
    // answer they would get for one that never existed.
    if state.adapter_connections.delete(&id, &ctx).await? == 0 {
        return Err(AdapterError::NotFound(format!(
            "Adapter connection not found: {id}"
        )));
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
        (status = 400, description = "Invalid request", body = crate::error::ErrorBody),
        (status = 404, description = "Connection or task not found", body = crate::error::ErrorBody),
    )
)]
pub async fn adapter_send(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<AdapterSendRequest>,
) -> Result<Json<AdapterSendResponse>, AdapterError> {
    let ctx = auth.scope_context();

    if let Some(task_id) = req.task_id.as_deref() {
        ensure_task_access(&state, &ctx, task_id).await?;
    }

    let connection =
        resolve_connection(&state, &ctx, req.adapter, req.connection_id.as_deref()).await?;

    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("adapters not configured".into()))?;
    let provider = providers
        .get(req.adapter)
        .ok_or_else(|| AdapterError::AdapterNotConfigured(req.adapter.as_str().to_string()))?;

    let files = resolve_files(&state, &req.file_ids).await;
    if !files.is_empty() && !provider.supports_files() {
        return Err(AdapterError::BadRequest(format!(
            "{} adapter does not support file attachments",
            req.adapter.as_str()
        )));
    }

    let reply = OutboundReply {
        text: truncate_text(&req.text, provider.max_message_length()),
        task_url: None,
        footer: None,
        author_user_id: None,
        is_error: false,
        files,
    };

    let result = provider
        .post_to_destination(&req.destination, &reply)
        .await?;

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        let binding = upsert_binding(
            &state,
            &connection,
            None,
            task_id,
            provider.connection_provider_key(),
            result.subject_kind,
            &result.subject_key,
            result.destination_config.clone(),
            ORIGIN_CONNECTED_THREAD,
            Some(result.message_external_id.clone()),
        )
        .await?;
        provider
            .on_binding_created(&state, &connection, &binding, task_id)
            .await;
        Some(binding.id)
    } else {
        None
    };

    Ok(Json(AdapterSendResponse {
        external_id: result.message_external_id,
        subject_kind: result.subject_kind.to_string(),
        binding_id,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/users/{login}/dm",
    tag = "users",
    params(("login" = String, Path, description = "Moatless user login")),
    request_body = SendDmRequest,
    responses(
        (status = 200, description = "DM sent", body = SendDmResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorBody),
        (status = 404, description = "User or provider mapping not found", body = crate::error::ErrorBody),
    )
)]
pub async fn send_user_dm(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(login): Path<String>,
    Json(req): Json<SendDmRequest>,
) -> Result<Json<SendDmResponse>, AdapterError> {
    let ctx = auth.scope_context();

    if let Some(task_id) = req.task_id.as_deref() {
        ensure_task_access(&state, &ctx, task_id).await?;
    }

    let target = state
        .users
        .get_user_by_login(&login)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("User {login} not found")))?;

    let provider_user_id = state
        .users
        .get_provider_id(&target.id, &req.provider)
        .await?
        .ok_or_else(|| {
            AdapterError::NotFound(format!(
                "User {login} has no {} provider id configured",
                req.provider
            ))
        })?;

    let kind = AdapterKind::parse(&req.provider).ok_or_else(|| {
        AdapterError::BadRequest(format!("Unknown adapter provider: {}", req.provider))
    })?;

    let connection = resolve_connection(&state, &ctx, kind, None).await?;

    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("adapters not configured".into()))?;
    let provider = providers
        .get(kind)
        .ok_or_else(|| AdapterError::AdapterNotConfigured(kind.as_str().to_string()))?;

    let destination = provider
        .dm_destination(&provider_user_id)
        .await?
        .ok_or_else(|| {
            AdapterError::BadRequest(format!(
                "User DM is not supported for adapter {}",
                kind.as_str()
            ))
        })?;

    let reply = OutboundReply {
        text: truncate_text(&req.text, provider.max_message_length()),
        task_url: None,
        footer: None,
        author_user_id: None,
        is_error: false,
        files: vec![],
    };

    let result = provider.post_to_destination(&destination, &reply).await?;

    let conversation_id = result
        .destination_config
        .get("channel")
        .and_then(|v| v.as_str())
        .or_else(|| {
            result
                .destination_config
                .get("chat_id")
                .and_then(|v| v.as_str())
        })
        .unwrap_or(provider_user_id.as_str())
        .to_string();

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        let binding = upsert_binding(
            &state,
            &connection,
            None,
            task_id,
            provider.connection_provider_key(),
            result.subject_kind,
            &result.subject_key,
            result.destination_config.clone(),
            ORIGIN_CONNECTED_THREAD,
            Some(result.message_external_id.clone()),
        )
        .await?;
        binding.id
    } else {
        String::new()
    };

    Ok(Json(SendDmResponse {
        binding_id,
        conversation_id,
        message_ts: result.message_external_id,
    }))
}
