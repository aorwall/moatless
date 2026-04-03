//! Channel HTTP handlers — webhook endpoints + config CRUD.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use tracing::{info, warn};

use crate::extractors::AuthUser;
use crate::state::AppState;

use cron::Schedule;

use super::config_dao::ChannelConfigPatch;
use super::error::ChannelError;
use super::inbound;
use super::provider::ChannelType;
use super::schemas::*;
use super::slack::provider::SlackWebhookResult;

// =========================================================================
// Provider discovery
// =========================================================================

/// List configured channel providers.
///
/// Returns the names of user-facing providers that are currently configured
/// (e.g. `["slack", "telegram"]`). Internal providers like webhook and cron
/// are excluded.
#[utoipa::path(
    get,
    path = "/api/v1/channels/providers",
    tag = "channels",
    responses(
        (status = 200, description = "Configured provider names", body = Vec<String>)
    )
)]
pub async fn list_providers(
    State(state): State<AppState>,
) -> Json<Vec<String>> {
    let types = state
        .channel_providers
        .as_ref()
        .map(|p| p.registered_types())
        .unwrap_or_default();

    Json(types.into_iter().map(|t| t.as_str().to_string()).collect())
}

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

    // Look up config — exact match first, then try wildcard auto-create
    let config = state.channel_configs.find_by_provider_source("slack", source_id)
        .await?;

    let config = match config {
        Some(c) if c.active => c,
        Some(_) => {
            info!(source_id, "Slack message ignored: channel config exists but is inactive");
            return Ok(StatusCode::OK.into_response());
        }
        None => {
            // Try to auto-create from wildcard config
            let source_name = slack_provider.get_channel_name(source_id).await.unwrap_or(None);
            match super::defaults::create_config_from_wildcard(
                &*state.channel_configs,
                "slack",
                source_id,
                source_name,
                None, // team_id not available
                "system",
            )
            .await
            {
                Ok(Some(c)) => {
                    info!(source_id, config_id = %c.id, "Slack channel auto-created from wildcard config");
                    c
                }
                Ok(None) => {
                    info!(source_id, "Slack message ignored: no channel config for this channel");
                    return Ok(StatusCode::OK.into_response());
                }
                Err(e) => {
                    warn!(source_id, error = %e, "Failed to auto-create Slack config from wildcard");
                    return Ok(StatusCode::OK.into_response());
                }
            }
        }
    };

    // Check if this is a reply in a thread we're already tracking (e.g. outbound-created).
    // If so, bypass the trigger filter so replies to our messages are always processed.
    let thread_ts = payload.pointer("/event/thread_ts").and_then(|v| v.as_str());
    let is_tracked_thread = if let Some(tts) = thread_ts {
        state
            .channel_threads
            .find_by_config_and_thread(&config.id, tts)
            .await
            .ok()
            .flatten()
            .is_some()
    } else {
        false
    };

    // Parse event with config context
    let result = slack_provider
        .parse_event(&headers, &body, &config, &state.db, is_tracked_thread)
        .await?;

    match result {
        SlackWebhookResult::Challenge(challenge) => {
            Ok((StatusCode::OK, challenge).into_response())
        }
        SlackWebhookResult::Message(msg) => {
            // Process in background (Slack requires <3s response)
            let state_clone = state.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    inbound::process_inbound(&state_clone, *msg).await
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
// Telegram webhook handler
// =========================================================================

/// Handle Telegram Bot API webhook update.
#[utoipa::path(
    post,
    path = "/api/v1/channels/telegram/webhook",
    tag = "channels",
    request_body(content = String, description = "Telegram Bot API Update payload"),
    responses(
        (status = 200, description = "Webhook processed"),
        (status = 401, description = "Invalid secret token")
    )
)]
pub async fn telegram_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, ChannelError> {
    use axum::response::IntoResponse;

    let providers = state
        .channel_providers
        .as_ref()
        .ok_or_else(|| ChannelError::ProviderNotConfigured("telegram".into()))?;

    let telegram = providers
        .get(ChannelType::Telegram)
        .ok_or_else(|| ChannelError::ProviderNotConfigured("telegram".into()))?;

    let telegram_provider = telegram
        .as_any()
        .downcast_ref::<super::telegram::provider::TelegramProvider>()
        .ok_or_else(|| ChannelError::Internal("Provider type mismatch".into()))?;

    info!("Telegram webhook received");

    // Webhook-specific: verify secret token header before processing.
    // Parse payload to find the matching config so we can check its secret.
    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| ChannelError::BadRequest(format!("Invalid JSON: {e}")))?;

    let message = payload.get("message").or_else(|| payload.get("channel_post"));
    if let Some(chat_id) = message
        .and_then(|m| m.pointer("/chat/id"))
        .and_then(serde_json::Value::as_i64)
    {
        let source_id = chat_id.to_string();
        let chat_type = message
            .and_then(|m| m.pointer("/chat/type"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let is_private_chat = chat_type == "private";

        let config = state
            .channel_configs
            .find_by_provider_source("telegram", &source_id)
            .await?;
        let config = match config {
            Some(c) if c.active => Some(c),
            None if is_private_chat => {
                state
                    .channel_configs
                    .find_by_provider_source("telegram", "*")
                    .await?
                    .filter(|c| c.active)
            }
            _ => None,
        };

        if let Some(ref cfg) = config {
            let expected_secret = cfg
                .provider_config
                .as_ref()
                .and_then(|c| c.get("secret_token"))
                .and_then(|v| v.as_str());

            if let Some(expected) = expected_secret {
                let provided = headers
                    .get("x-telegram-bot-api-secret-token")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");

                if provided != expected {
                    return Err(ChannelError::SignatureInvalid);
                }
            }
        }
    }

    // Delegate config resolution + parsing + routing to shared helper
    if let Err(e) = super::telegram::provider::resolve_and_process_telegram_update(
        &state,
        telegram_provider,
        &body,
    )
    .await
    {
        warn!(error = %e, "Telegram webhook processing error");
    }

    Ok(StatusCode::OK.into_response())
}

// =========================================================================
// Telegram pairing endpoints
// =========================================================================

/// List all pending pairing requests across all channels owned by the
/// authenticated user. Unlike `list_pairing_requests`, this does not require
/// a specific `config_id`.
#[utoipa::path(
    get,
    path = "/api/v1/channels/pairing/my-requests",
    tag = "channels",
    responses(
        (status = 200, description = "Pending pairing requests", body = Vec<PairingRequestResponse>)
    )
)]
pub async fn list_my_pairing_requests(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<PairingRequestResponse>>, ChannelError> {
    let requests =
        super::telegram::pairing::list_pending_for_user(&state.db, &auth.user_id).await?;
    let items = requests.iter().map(pairing_request_from_model).collect();
    Ok(Json(items))
}

/// List pending pairing requests for a Telegram channel config.
#[utoipa::path(
    get,
    path = "/api/v1/channels/telegram/pairing/requests",
    tag = "channels",
    params(
        ("config_id" = String, Query, description = "Channel config ID")
    ),
    responses(
        (status = 200, description = "Pending pairing requests"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Config not found")
    )
)]
pub async fn list_pairing_requests(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<PairingQuery>,
) -> Result<Json<Vec<serde_json::Value>>, ChannelError> {
    let config = state
        .channel_configs
        .find_by_id(&query.config_id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(query.config_id.clone()))?;

    // Only the config owner can view pairing requests
    if config.created_by != auth.user_id {
        return Err(ChannelError::AccessDenied(
            "Only the channel config owner can view pairing requests".into(),
        ));
    }

    let requests =
        super::telegram::pairing::list_pending(&state.db, &query.config_id).await?;

    let items: Vec<serde_json::Value> = requests
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "configId": r.config_id,
                "providerUserId": r.provider_user_id,
                "code": r.code,
                "status": r.status,
                "meta": r.meta,
                "createdAt": r.created_at.to_rfc3339(),
                "updatedAt": r.updated_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(items))
}

/// Approve a Telegram pairing request by code.
#[utoipa::path(
    post,
    path = "/api/v1/channels/telegram/pairing/approve",
    tag = "channels",
    request_body = ApprovePairingRequest,
    responses(
        (status = 200, description = "Pairing approved"),
        (status = 400, description = "Invalid code"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Config not found")
    )
)]
pub async fn approve_pairing(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ApprovePairingRequest>,
) -> Result<Json<serde_json::Value>, ChannelError> {
    let config = state
        .channel_configs
        .find_by_id(&req.config_id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(req.config_id.clone()))?;

    // Only the config owner can approve
    if config.created_by != auth.user_id {
        return Err(ChannelError::AccessDenied(
            "Only the channel config owner can approve pairing requests".into(),
        ));
    }

    let approved =
        super::telegram::pairing::approve_request(&state.db, &req.config_id, &req.code).await?;

    Ok(Json(serde_json::json!({
        "id": approved.id,
        "configId": approved.config_id,
        "providerUserId": approved.provider_user_id,
        "code": approved.code,
        "status": approved.status,
        "meta": approved.meta,
        "createdAt": approved.created_at.to_rfc3339(),
        "updatedAt": approved.updated_at.to_rfc3339(),
    })))
}

// =========================================================================
// GitHub PR webhook handler
// =========================================================================

/// Handle GitHub webhook for PR events.
#[utoipa::path(
    post,
    path = "/api/v1/channels/github-pr/webhook",
    tag = "channels",
    request_body(content = String, description = "GitHub webhook payload"),
    responses(
        (status = 200, description = "Webhook processed"),
        (status = 401, description = "Invalid signature")
    )
)]
pub async fn github_pr_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ChannelError> {
    let providers = state
        .channel_providers
        .as_ref()
        .ok_or_else(|| ChannelError::ProviderNotConfigured("github_pr".into()))?;

    let github_pr = providers
        .get(ChannelType::GithubPr)
        .ok_or_else(|| ChannelError::ProviderNotConfigured("github_pr".into()))?;

    let github_pr_provider = github_pr
        .as_any()
        .downcast_ref::<super::github_pr::provider::GithubPrProvider>()
        .ok_or_else(|| ChannelError::Internal("Provider type mismatch".into()))?;

    if let Err(e) = super::github_pr::provider::resolve_and_process_github_webhook(
        &state,
        github_pr_provider,
        &headers,
        &body,
    )
    .await
    {
        warn!(error = %e, "GitHub PR webhook processing error");
    }

    Ok(StatusCode::OK)
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

    match inbound::process_inbound(&state, msg).await {
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
        (status = 400, description = "Slack not configured", body = crate::error::ErrorBody)
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
        (status = 400, description = "Invalid request", body = crate::error::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn create_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateChannelConfigRequest>,
) -> Result<Json<ChannelConfigResponse>, ChannelError> {
    info!(provider = %req.provider, source_id = %req.source_id, "Creating channel config");

    // Validate cron schedule if applicable
    validate_cron_schedule(req.provider.as_str(), req.provider_config.as_ref())?;

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

    // Auto-resolve channel name from provider if not supplied
    let source_name = match req.source_name {
        Some(name) => Some(name),
        None => resolve_channel_name(&state, req.provider, &req.source_id).await,
    };

    let id = nanoid::nanoid!(21);
    let now = chrono::Utc::now();

    let scope = req.scope.unwrap_or(crate::scope::Scope::User);

    let row = super::models::ChannelConfig {
        id: id.clone(),
        provider: req.provider.as_str().to_string(),
        source_id: req.source_id,
        source_name,
        repository_id: req.repository_id,
        routing_mode: req.routing_mode,
        prompt_template: req.prompt_template,
        active: true,
        provider_config: req.provider_config,
        ongoing_task_id: None,
        response_config_id: req.response_config_id,
        scope,
        team_id: req.team_id,
        task_name: req.task_name,
        tag_ids: req.tag_ids.unwrap_or_default(),
        run_as_user_id: req.run_as_user_id,
        created_by: auth.user_id,
        created_at: now,
        updated_at: now,
    };

    state.channel_configs.insert(&row).await?;
    info!(config_id = %id, provider = %row.provider, "Channel config created");

    Ok(Json(channel_config_from_model(&row)))
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
    auth: AuthUser,
    Query(query): Query<ListChannelConfigsQuery>,
) -> Result<Json<Vec<ChannelConfigResponse>>, ChannelError> {
    info!(provider = ?query.provider, repository_id = ?query.repository_id, "Listing channel configs");

    let rows = state.channel_configs.list_scoped(
        query.provider.as_deref(),
        query.repository_id.as_deref(),
        false, // show all, not just active
        &auth.scope_context(),
    )
    .await?;

    let items: Vec<ChannelConfigResponse> = rows.iter().map(channel_config_from_model).collect();
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
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ChannelConfigResponse>, ChannelError> {
    info!(config_id = %id, "Getting channel config");

    let row = state.channel_configs.find_by_id_scoped(&id, &auth.scope_context())
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(id))?;

    Ok(Json(channel_config_from_model(&row)))
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
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn update_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateChannelConfigRequest>,
) -> Result<Json<ChannelConfigResponse>, ChannelError> {
    info!(config_id = %id, "Updating channel config");

    // Verify exists and accessible
    let existing = state.channel_configs.find_by_id_scoped(&id, &auth.scope_context())
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(id.clone()))?;

    // Validate cron schedule if updating provider_config on a cron channel
    validate_cron_schedule(&existing.provider, req.provider_config.as_ref())?;

    let patch = ChannelConfigPatch {
        source_name: req.source_name,
        repository_id: req.repository_id,
        routing_mode: req.routing_mode,
        prompt_template: req.prompt_template,
        active: req.active,
        provider_config: req.provider_config,
        response_config_id: req.response_config_id,
        task_name: req.task_name,
        tag_ids: req.tag_ids,
        run_as_user_id: req.run_as_user_id,
        ..Default::default()
    };

    state.channel_configs.update(&id, &patch).await?;

    let row = state.channel_configs.find_by_id(&id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(id))?;

    Ok(Json(channel_config_from_model(&row)))
}

/// Manually trigger a channel config, creating a task exactly as if the provider fired.
///
/// Works for any provider type (cron, slack, github_pr, etc.) and does not require
/// the channel to be active. The created task will have `responseConfigId` wired up
/// so outbound replies flow back to the configured channel (e.g. Slack).
#[utoipa::path(
    post,
    path = "/api/v1/channels/configs/{id}/trigger",
    tag = "channels",
    params(
        ("id" = String, Path, description = "Channel config ID")
    ),
    request_body = TriggerChannelRequest,
    responses(
        (status = 200, description = "Task created", body = TriggerChannelResponse),
        (status = 403, description = "Access denied", body = crate::error::ErrorBody),
        (status = 404, description = "Config not found", body = crate::error::ErrorBody)
    )
)]
pub async fn trigger_channel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<TriggerChannelRequest>,
) -> Result<Json<TriggerChannelResponse>, ChannelError> {
    let config = state
        .channel_configs
        .find_by_id(&id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(id.clone()))?;

    // Only the config owner can trigger it manually
    if config.created_by != auth.user_id {
        return Err(ChannelError::AccessDenied(
            "Only the channel config owner can trigger it".into(),
        ));
    }

    let task_id =
        inbound::trigger_inbound(&state, &config, req.message.as_deref(), req.thread_properties)
            .await?;

    let task = state
        .tasks
        .find_by_id_internal(&task_id)
        .await
        .map_err(|e| ChannelError::Internal(format!("Failed to look up task: {e}")))?
        .ok_or_else(|| ChannelError::Internal("Task not found after creation".into()))?;

    Ok(Json(TriggerChannelResponse {
        task_id,
        short_id: task.short_id.unwrap_or_default(),
    }))
}

// =========================================================================
// Channel defaults (wildcard config) handlers
// =========================================================================

/// Get channel defaults for a provider.
///
/// Returns the wildcard (`source_id="*"`) config that serves as the default
/// template for auto-creating per-channel configs.
#[utoipa::path(
    get,
    path = "/api/v1/channels/defaults/{provider}",
    tag = "channels",
    params(
        ("provider" = String, Path, description = "Channel provider (slack, telegram)"),
        ("teamId" = Option<String>, Query, description = "Team ID (required if user belongs to multiple teams)")
    ),
    responses(
        (status = 200, description = "Default config", body = ChannelConfigResponse),
        (status = 404, description = "No defaults configured", body = crate::error::ErrorBody)
    )
)]
pub async fn get_defaults(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider): Path<String>,
    Query(query): Query<ChannelDefaultsQuery>,
) -> Result<Json<ChannelConfigResponse>, ChannelError> {
    let team_id = match query.team_id.as_deref() {
        Some(tid) => Some(tid),
        None if auth.team_ids.len() == 1 => Some(auth.team_ids[0].as_str()),
        None if auth.team_ids.len() > 1 => {
            return Err(ChannelError::BadRequest(
                "You belong to multiple teams. Please specify teamId.".to_string(),
            ));
        }
        None => None,
    };
    let row = state
        .channel_configs
        .find_wildcard(&provider, team_id)
        .await?
        .ok_or_else(|| {
            ChannelError::ConfigNotFound(format!("No defaults for provider: {provider}"))
        })?;

    Ok(Json(channel_config_from_model(&row)))
}

/// Set channel defaults for a provider.
///
/// Creates or updates the wildcard (`source_id="*"`) config for the given
/// provider. New channels of this provider will inherit these settings.
#[utoipa::path(
    put,
    path = "/api/v1/channels/defaults/{provider}",
    tag = "channels",
    params(
        ("provider" = String, Path, description = "Channel provider (slack, telegram)")
    ),
    request_body = SetChannelDefaultsRequest,
    responses(
        (status = 200, description = "Defaults saved", body = ChannelConfigResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorBody)
    )
)]
pub async fn set_defaults(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider): Path<String>,
    Json(req): Json<SetChannelDefaultsRequest>,
) -> Result<Json<ChannelConfigResponse>, ChannelError> {
    info!(provider = %provider, "Setting channel defaults");

    // Validate cron schedule if applicable
    validate_cron_schedule(&provider, req.provider_config.as_ref())?;

    // Validate repository exists
    state
        .repositories
        .find_repository_by_id(&req.repository_id)
        .await?
        .ok_or_else(|| {
            ChannelError::BadRequest(format!("Repository not found: {}", req.repository_id))
        })?;

    let team_id = match req.team_id.as_deref() {
        Some(tid) => Some(tid),
        None if auth.team_ids.len() == 1 => Some(auth.team_ids[0].as_str()),
        None if auth.team_ids.len() > 1 => {
            return Err(ChannelError::BadRequest(
                "You belong to multiple teams. Please specify teamId.".to_string(),
            ));
        }
        None => None,
    };

    // Check if wildcard config already exists
    if let Some(existing) = state.channel_configs.find_wildcard(&provider, team_id).await? {
        // Update existing wildcard config
        let patch = ChannelConfigPatch {
            repository_id: Some(req.repository_id),
            routing_mode: Some(req.routing_mode),
            prompt_template: Some(req.prompt_template),
            provider_config: req.provider_config,
            response_config_id: Some(req.response_config_id),
            task_name: Some(req.task_name),
            tag_ids: req.tag_ids,
            ..Default::default()
        };

        state.channel_configs.update(&existing.id, &patch).await?;

        let row = state
            .channel_configs
            .find_by_id(&existing.id)
            .await?
            .ok_or_else(|| ChannelError::ConfigNotFound(existing.id))?;

        Ok(Json(channel_config_from_model(&row)))
    } else {
        // Create new wildcard config
        let id = nanoid::nanoid!(21);
        let now = chrono::Utc::now();

        let row = super::models::ChannelConfig {
            id: id.clone(),
            provider: provider.clone(),
            source_id: "*".to_string(),
            source_name: None,
            repository_id: req.repository_id,
            routing_mode: req.routing_mode,
            prompt_template: req.prompt_template,
            active: true,
            provider_config: req.provider_config,
            ongoing_task_id: None,
            response_config_id: req.response_config_id,
            scope: crate::scope::Scope::Global,
            team_id: team_id.map(str::to_string),
            task_name: req.task_name,
            tag_ids: req.tag_ids.unwrap_or_default(),
            run_as_user_id: None,
            created_by: auth.user_id,
            created_at: now,
            updated_at: now,
        };

        state.channel_configs.insert(&row).await?;
        info!(config_id = %id, provider = %provider, "Channel defaults created");

        Ok(Json(channel_config_from_model(&row)))
    }
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
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ChannelError> {
    info!(config_id = %id, "Deleting channel config");

    state.channel_configs.find_by_id_scoped(&id, &auth.scope_context())
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(id.clone()))?;

    state.channel_configs.delete(&id).await?;

    Ok(StatusCode::NO_CONTENT)
}

// =========================================================================
// Task channel endpoints (list + send)
// =========================================================================

/// List channels connected to a task.
///
/// Returns the channel configs linked to the task via channel threads.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/channels",
    tag = "channels",
    params(
        ("task_id" = String, Path, description = "Task ID")
    ),
    responses(
        (status = 200, description = "Channels for task", body = Vec<TaskChannelResponse>),
        (status = 404, description = "No channels for task", body = crate::error::ErrorBody)
    )
)]
pub async fn list_task_channels(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<Vec<TaskChannelResponse>>, ChannelError> {
    let threads = state.channel_threads.find_by_task_id(&task_id).await?;

    let mut channels = Vec::new();
    for thread in &threads {
        if let Ok(Some(config)) = state.channel_configs.find_by_id(&thread.config_id).await {
            channels.push(super::schemas::task_channel_from_model(&config));
        }
    }

    Ok(Json(channels))
}

/// Send a message to all channels connected to a task.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/channels/send",
    tag = "channels",
    params(
        ("task_id" = String, Path, description = "Task ID")
    ),
    request_body = SendChannelMessageRequest,
    responses(
        (status = 200, description = "Message sent", body = SendChannelMessageResponse),
        (status = 404, description = "No channels for task", body = crate::error::ErrorBody),
        (status = 502, description = "All providers failed", body = crate::error::ErrorBody)
    )
)]
pub async fn send_to_task_channels(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SendChannelMessageRequest>,
) -> Result<Json<SendChannelMessageResponse>, ChannelError> {
    let providers = state
        .channel_providers
        .as_ref()
        .ok_or_else(|| ChannelError::ProviderNotConfigured("No channel providers configured".into()))?;

    let (sent_count, errors) =
        super::outbound::send_to_channels(&state, providers, &task_id, &req.text, &req.file_ids, None).await?;

    if sent_count == 0 && !errors.is_empty() {
        return Err(ChannelError::ProviderApi(errors.join("; ")));
    }

    Ok(Json(SendChannelMessageResponse { sent_count, errors }))
}

/// Send a message to a specific channel connected to a task.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/channels/{config_id}/send",
    tag = "channels",
    params(
        ("task_id" = String, Path, description = "Task ID"),
        ("config_id" = String, Path, description = "Channel config ID")
    ),
    request_body = SendChannelMessageRequest,
    responses(
        (status = 200, description = "Message sent", body = SendChannelMessageResponse),
        (status = 404, description = "Channel not found for task", body = crate::error::ErrorBody),
        (status = 502, description = "Provider failed", body = crate::error::ErrorBody)
    )
)]
pub async fn send_to_task_channel(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path((task_id, config_id)): Path<(String, String)>,
    Json(req): Json<SendChannelMessageRequest>,
) -> Result<Json<SendChannelMessageResponse>, ChannelError> {
    let providers = state
        .channel_providers
        .as_ref()
        .ok_or_else(|| ChannelError::ProviderNotConfigured("No channel providers configured".into()))?;

    let (sent_count, errors) =
        super::outbound::send_to_channels(&state, providers, &task_id, &req.text, &req.file_ids, Some(&config_id))
            .await?;

    if sent_count == 0 && !errors.is_empty() {
        return Err(ChannelError::ProviderApi(errors.join("; ")));
    }

    Ok(Json(SendChannelMessageResponse { sent_count, errors }))
}

/// Validate cron schedule in provider_config if the provider is "cron".
///
/// Best-effort channel name resolution from the provider API.
///
/// Currently only Slack is supported. Returns `None` for other providers or on error.
async fn resolve_channel_name(
    state: &AppState,
    provider: ChannelType,
    source_id: &str,
) -> Option<String> {
    if provider != ChannelType::Slack {
        return None;
    }

    let providers = state.channel_providers.as_ref()?;
    let slack = providers.get(ChannelType::Slack)?;
    let slack_provider = slack
        .as_any()
        .downcast_ref::<super::slack::provider::SlackProvider>()?;

    match slack_provider.get_channel_name(source_id).await {
        Ok(name) => name,
        Err(e) => {
            warn!(source_id, error = %e, "Failed to resolve Slack channel name");
            None
        }
    }
}

/// The `cron` crate expects 7-field expressions: `sec min hour dom month dow year`.
/// Returns `BadRequest` if the schedule field is present but cannot be parsed.
fn validate_cron_schedule(
    provider: &str,
    provider_config: Option<&serde_json::Value>,
) -> Result<(), ChannelError> {
    if provider != "cron" {
        return Ok(());
    }

    let Some(schedule_str) = provider_config
        .and_then(|c| c.get("schedule"))
        .and_then(|v| v.as_str())
    else {
        return Ok(());
    };

    schedule_str.parse::<Schedule>().map_err(|e| {
        ChannelError::BadRequest(format!(
            "Invalid cron schedule \"{schedule_str}\": {e}. \
             The cron library requires 7-field expressions: sec min hour dom month dow year \
             (e.g. \"0 0 6 * * MON-FRI *\")"
        ))
    })?;

    Ok(())
}
