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

    let event_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
    info!(event_type, "Slack webhook received");

    // Handle url_verification before config lookup (no channel info needed)
    if event_type == "url_verification" {
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
        let event_subtype = payload.pointer("/event/type").and_then(|v| v.as_str()).unwrap_or("unknown");
        warn!(event_type, event_subtype, "Slack webhook has no channel ID in event payload, ignoring");
        return Ok(StatusCode::OK.into_response());
    }

    // Look up config
    let config = state.channel_configs.find_by_provider_source("slack", source_id)
        .await?;

    let config = match config {
        Some(c) if c.active => c,
        Some(_) => {
            info!(source_id, "Slack message ignored: channel config exists but is inactive");
            return Ok(StatusCode::OK.into_response());
        }
        None => {
            info!(source_id, "Slack message ignored: no channel config for this channel");
            return Ok(StatusCode::OK.into_response());
        }
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
        SlackWebhookResult::Ignore => {
            info!(source_id, "Slack event ignored by provider (bot message, unsupported event, or deduplicated)");
            Ok(StatusCode::OK.into_response())
        }
    }
}

// =========================================================================
// Webhook inbound handler
// =========================================================================

/// Handle inbound webhook message.
///
/// Receives a JSON payload with `text`, optional `threadId`, `userId`, and `metadata`.
/// Verifies HMAC signature if `webhook_secret` is configured in the channel config.
#[utoipa::path(
    post,
    path = "/api/v1/channels/webhook/{config_id}",
    tag = "channels",
    params(
        ("config_id" = String, Path, description = "Channel config ID")
    ),
    request_body = super::webhook::provider::WebhookPayload,
    responses(
        (status = 200, description = "Webhook message accepted"),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Invalid signature"),
        (status = 404, description = "Config not found")
    )
)]
pub async fn webhook_inbound(
    State(state): State<AppState>,
    Path(config_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, ChannelError> {
    use axum::response::IntoResponse;

    info!(config_id = %config_id, "Webhook inbound received");

    // Look up config by ID
    let config = state
        .channel_configs
        .find_by_id(&config_id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(config_id.clone()))?;

    if !config.active {
        info!(config_id = %config_id, "Webhook ignored: config is inactive");
        return Ok(StatusCode::OK.into_response());
    }

    if config.provider != "webhook" {
        return Err(ChannelError::BadRequest(format!(
            "Config {} is not a webhook config (provider={})",
            config_id, config.provider
        )));
    }

    // Extract signature header
    let signature = headers
        .get("x-webhook-signature")
        .and_then(|v| v.to_str().ok());

    // Parse inbound message
    let msg =
        super::webhook::provider::WebhookProvider::parse_inbound(&config, &body, signature)?;

    let providers = state
        .channel_providers
        .as_ref()
        .ok_or_else(|| ChannelError::ProviderNotConfigured("webhook".into()))?
        .clone();

    match inbound::process_inbound(&state, &providers, msg).await {
        Ok(task_id) => Ok(Json(serde_json::json!({ "taskId": task_id })).into_response()),
        Err(e) => {
            warn!(error = %e, "Failed to process webhook inbound message");
            Err(e)
        }
    }
}

// =========================================================================
// Slack channel discovery
// =========================================================================

/// List available Slack channels the bot is a member of.
#[utoipa::path(
    get,
    path = "/api/v1/channels/slack/channels",
    tag = "channels",
    responses(
        (status = 200, description = "Available Slack channels", body = Vec<SlackChannelInfoResponse>),
        (status = 400, description = "Slack not configured", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn list_slack_channels(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<SlackChannelInfoResponse>>, ChannelError> {
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

    let channels = slack_provider.list_channels().await?;

    let items: Vec<SlackChannelInfoResponse> = channels
        .into_iter()
        .map(|ch| SlackChannelInfoResponse {
            id: ch.id,
            name: ch.name,
        })
        .collect();

    Ok(Json(items))
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
    info!(provider = %req.provider, source_id = %req.source_id, "Creating channel config");

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
        response_config_id: req.response_config_id,
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
    info!(provider = ?query.provider, repository_id = ?query.repository_id, "Listing channel configs");

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
    info!(config_id = %id, "Getting channel config");

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
    info!(config_id = %id, "Updating channel config");

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
        response_config_id: req.response_config_id,
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
    info!(config_id = %id, "Deleting channel config");

    state.channel_configs.find_by_id(&id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(id.clone()))?;

    state.channel_configs.delete(&id).await?;

    Ok(StatusCode::NO_CONTENT)
}
