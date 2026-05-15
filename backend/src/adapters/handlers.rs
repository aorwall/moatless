use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::adapters::{
    AdapterConnectionResponse, AdapterSendRequest, AdapterSendResponse, CreateAdapterConnectionRequest,
};
use moatless_api_schemas::user::{SendDmRequest, SendDmResponse};

use super::common::{
    adapter_connection_response, ensure_can_write_adapter_resource, ensure_task_access,
    resolve_connection, resolve_scope_for_adapter_create, scoped_resource, AdapterFilter,
};
use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, OutboundReply};
use crate::adapters::slack::adapter::SlackAdapter;
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
    let rows = PgAdapterConnectionRepository(state.db.clone())
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

    let now = Utc::now();
    let provider = req.adapter_kind.as_str().to_string();
    let repo = PgAdapterConnectionRepository(state.db.clone());
    if let Some(existing) = repo
        .find_by_external_account(&provider, &req.external_account_id)
        .await?
    {
        let resource = scoped_resource(
            existing.scope,
            existing.team_id.as_deref(),
            existing.owner_user_id.as_deref(),
            &existing.created_by,
        );
        ensure_can_write_adapter_resource(&state, &auth, &resource, "adapter connection").await?;
    }
    let resolved =
        resolve_scope_for_adapter_create(&state, &auth, req.scope, req.team_id.clone()).await?;
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
        created_by: auth.user_id,
        scope: resolved.scope,
        team_id: Some(resolved.team_id),
        owner_user_id: resolved.owner_user_id,
        created_at: now,
        updated_at: now,
    };
    let saved = repo.upsert(&row).await?;
    Ok(Json(adapter_connection_response(&saved)))
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
    let provider = providers.get(req.adapter).ok_or_else(|| {
        AdapterError::AdapterNotConfigured(req.adapter.as_str().to_string())
    })?;

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
        if req.adapter == AdapterKind::Linear
            && result.subject_kind == crate::adapters::linear::SUBJECT_KIND_ISSUE
        {
            link_moatless_task_to_linear_issue(
                &state,
                provider.as_ref(),
                &connection,
                &result.subject_key,
                task_id,
            )
            .await;
        }
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

/// Fire the moatless-task attachment after a Linear destination send has
/// bound to a task. Looks up the issue UUID in the binding result and
/// calls `attachmentLinkURL` via the `LinearAdapter`. Failures are logged
/// and swallowed so they don't unwind the surrounding send.
async fn link_moatless_task_to_linear_issue(
    state: &AppState,
    provider: &dyn crate::adapters::Adapter,
    connection: &crate::adapters::models::AdapterConnection,
    issue_uuid: &str,
    task_id: &str,
) {
    let Some(linear) = provider
        .as_any()
        .downcast_ref::<crate::adapters::linear::adapter::LinearAdapter>()
    else {
        tracing::warn!("Linear adapter downcast failed in adapter_send attachment hook");
        return;
    };
    let api = linear.agent_api_for_org(&connection.scope_value);
    let url = crate::adapters::linear::agent_runtime::task_url(state, task_id);
    let title = crate::adapters::linear::agent_runtime::task_attachment_title(state, task_id).await;
    if let Err(e) = api.attachment_link_url(issue_uuid, &url, &title).await {
        tracing::warn!(
            error = %e,
            task_id = %task_id,
            issue = %issue_uuid,
            "Failed to attach Moatless task URL to Linear issue"
        );
    }
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
        .await
        .map_err(|e| AdapterError::Internal(format!("Failed to look up user: {e}")))?
        .ok_or_else(|| AdapterError::NotFound(format!("User {login} not found")))?;

    let provider_user_id = state
        .users
        .get_provider_id(&target.id, &req.provider)
        .await
        .map_err(|e| AdapterError::Internal(format!("Failed to look up provider id: {e}")))?
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

    let destination = match kind {
        AdapterKind::Slack => {
            let slack = provider
                .as_any()
                .downcast_ref::<SlackAdapter>()
                .ok_or_else(|| {
                    AdapterError::Internal("Slack adapter downcast failed".into())
                })?;
            let dm_channel = slack.open_dm(&provider_user_id).await?;
            format!("dm:{dm_channel}")
        }
        AdapterKind::Telegram => format!("chat:{provider_user_id}"),
        _ => {
            return Err(AdapterError::BadRequest(format!(
                "User DM is not supported for adapter {}",
                kind.as_str()
            )));
        }
    };

    let reply = OutboundReply {
        text: truncate_text(&req.text, provider.max_message_length()),
        task_url: None,
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
