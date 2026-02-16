//! Channel HTTP handlers — webhook endpoints + config CRUD.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use tracing::{info, warn};

use super::config_dao::ChannelConfigPatch;
use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::ChannelError;
use super::inbound;
use super::provider::ChannelType;
use super::slack::provider::SlackWebhookResult;
use super::schemas::*;

// =========================================================================
// Webhook handlers
// =========================================================================

/// Handle Slack Events API webhook.
#[utoipa::path(
    post,
    path = "/api/v1/channels/slack/webhook",
    tag = "channels",
    request_body(content = String, description = "Slack Events API payload"),
    responses(
        (status = 200, description = "Webhook processed"),
        (status = 401, description = "Invalid signature")
    )
)]
pub async fn slack_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, ChannelError> {
    use axum::response::IntoResponse;

    let providers = state
        .channel_providers
        .as_ref()
        .ok_or_else(|| ChannelError::ProviderNotConfigured("slack".into()))?;

    let slack = providers
        .get(ChannelType::Slack)
        .ok_or_else(|| ChannelError::ProviderNotConfigured("slack".into()))?;

    let slack_provider = slack
        .as_any()
        .downcast_ref::<super::slack::provider::SlackProvider>()
        .ok_or_else(|| ChannelError::Internal("Provider type mismatch".into()))?;

    // Quick-parse to get the channel ID for config lookup
    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| ChannelError::BadRequest(format!("Invalid JSON: {e}")))?;

    // Handle url_verification before config lookup (no channel info needed)
    if payload.get("type").and_then(|v| v.as_str()) == Some("url_verification") {
        // Still verify signature
        let timestamp = headers
            .get("x-slack-request-timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let sig = headers
            .get("x-slack-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        super::slack::signature::verify_signature(
            slack_provider.signing_secret(),
            timestamp,
            &body,
            sig,
        )?;

        let challenge = payload
            .get("challenge")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Ok((StatusCode::OK, challenge.to_string()).into_response());
    }

    // Extract source_id (channel) from event
    let source_id = payload
        .pointer("/event/channel")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if source_id.is_empty() {
        return Ok(StatusCode::OK.into_response());
    }

    // Look up config
    let config = state.channel_configs.find_by_provider_source("slack", source_id)
        .await?;

    let config = match config {
        Some(c) if c.active => c,
        _ => return Ok(StatusCode::OK.into_response()), // No config, ignore
    };

    // Parse event with config context
    let result = slack_provider
        .parse_event(&headers, &body, &config, &state.db)
        .await?;

    match result {
        SlackWebhookResult::Challenge(challenge) => {
            Ok((StatusCode::OK, challenge).into_response())
        }
        SlackWebhookResult::Message(msg) => {
            // Process in background (Slack requires <3s response)
            let state_clone = state.clone();
            let providers_clone = providers.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    inbound::process_inbound(&state_clone, &providers_clone, msg).await
                {
                    warn!(error = %e, "Failed to process Slack inbound message");
                }
            });
            Ok(StatusCode::OK.into_response())
        }
        SlackWebhookResult::Ignore => Ok(StatusCode::OK.into_response()),
    }
}

// =========================================================================
// Config CRUD handlers
// =========================================================================

/// Create a new channel config.
#[utoipa::path(
    post,
    path = "/api/v1/channels/configs",
    tag = "channels",
    request_body = CreateChannelConfigRequest,
    responses(
        (status = 200, description = "Config created", body = ChannelConfigResponse),
        (status = 400, description = "Invalid request", body = crate::sandbox::schemas::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn create_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateChannelConfigRequest>,
) -> Result<Json<ChannelConfigResponse>, ChannelError> {
    // Validate repository exists
    state.repositories.find_repository_by_id(&req.repository_id)
        .await?
        .ok_or_else(|| {
            ChannelError::BadRequest(format!("Repository not found: {}", req.repository_id))
        })?;

    // Check for existing config with same provider + source_id
    if state.channel_configs.find_by_provider_source(req.provider.as_str(), &req.source_id)
        .await?
        .is_some()
    {
        return Err(ChannelError::BadRequest(format!(
            "Config already exists for {} + {}",
            req.provider, req.source_id
        )));
    }

    let id = nanoid::nanoid!(21);
    let now = chrono::Utc::now();

    let row = super::models::ChannelConfig {
        id: id.clone(),
        provider: req.provider.as_str().to_string(),
        source_id: req.source_id,
        source_name: req.source_name,
        repository_id: req.repository_id,
        routing_mode: req.routing_mode,
        prompt_template: req.prompt_template,
        active: true,
        provider_config: req.provider_config,
        ongoing_task_id: None,
        created_by: auth.user_id,
        created_at: now,
        updated_at: now,
    };

    state.channel_configs.insert(&row).await?;
    info!(config_id = %id, provider = %row.provider, "Channel config created");

    Ok(Json(ChannelConfigResponse::from_model(&row)))
}

/// List channel configs with optional filters.
#[utoipa::path(
    get,
    path = "/api/v1/channels/configs",
    tag = "channels",
    params(
        ("provider" = Option<String>, Query, description = "Filter by provider"),
        ("repositoryId" = Option<String>, Query, description = "Filter by repository")
    ),
    responses(
        (status = 200, description = "Config list", body = Vec<ChannelConfigResponse>)
    )
)]
pub async fn list_configs(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<ListChannelConfigsQuery>,
) -> Result<Json<Vec<ChannelConfigResponse>>, ChannelError> {
    let rows = state.channel_configs.list(
        query.provider.as_deref(),
        query.repository_id.as_deref(),
        false, // show all, not just active
    )
    .await?;

    let items: Vec<ChannelConfigResponse> = rows.iter().map(ChannelConfigResponse::from_model).collect();
    Ok(Json(items))
}

/// Get a channel config by ID.
#[utoipa::path(
    get,
    path = "/api/v1/channels/configs/{id}",
    tag = "channels",
    params(
        ("id" = String, Path, description = "Channel config ID")
    ),
    responses(
        (status = 200, description = "Config details", body = ChannelConfigResponse),
        (status = 404, description = "Not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn get_config(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ChannelConfigResponse>, ChannelError> {
    let row = state.channel_configs.find_by_id(&id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(id))?;

    Ok(Json(ChannelConfigResponse::from_model(&row)))
}

/// Update a channel config.
#[utoipa::path(
    put,
    path = "/api/v1/channels/configs/{id}",
    tag = "channels",
    params(
        ("id" = String, Path, description = "Channel config ID")
    ),
    request_body = UpdateChannelConfigRequest,
    responses(
        (status = 200, description = "Config updated", body = ChannelConfigResponse),
        (status = 404, description = "Not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn update_config(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateChannelConfigRequest>,
) -> Result<Json<ChannelConfigResponse>, ChannelError> {
    // Verify exists
    state.channel_configs.find_by_id(&id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(id.clone()))?;

    let patch = ChannelConfigPatch {
        source_name: req.source_name,
        routing_mode: req.routing_mode,
        prompt_template: req.prompt_template,
        active: req.active,
        provider_config: req.provider_config,
        ..Default::default()
    };

    state.channel_configs.update(&id, &patch).await?;

    let row = state.channel_configs.find_by_id(&id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(id))?;

    Ok(Json(ChannelConfigResponse::from_model(&row)))
}

/// Delete a channel config.
#[utoipa::path(
    delete,
    path = "/api/v1/channels/configs/{id}",
    tag = "channels",
    params(
        ("id" = String, Path, description = "Channel config ID")
    ),
    responses(
        (status = 204, description = "Config deleted"),
        (status = 404, description = "Not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn delete_config(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ChannelError> {
    state.channel_configs.find_by_id(&id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(id.clone()))?;

    state.channel_configs.delete(&id).await?;
    info!(config_id = %id, "Channel config deleted");

    Ok(StatusCode::NO_CONTENT)
}
