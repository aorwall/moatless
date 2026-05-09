use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use tracing::{debug, info, warn};

use crate::channel::dao::bindings::PgTaskBindingRepository;
use crate::channel::error::ChannelError;
use crate::channel::provider::{ChannelProvider, ChannelType};
use crate::channel::services::provider_event_delivery::ProviderEventDeliveryService;
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/v1/integrations/slack/webhook",
    tag = "integrations",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
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
        .downcast_ref::<super::provider::SlackProvider>()
        .ok_or_else(|| ChannelError::Internal("Provider type mismatch".into()))?;

    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| ChannelError::BadRequest(format!("Invalid JSON: {e}")))?;

    let event_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    info!(event_type, "Slack webhook received");

    if event_type == "url_verification" {
        let timestamp = headers
            .get("x-slack-request-timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let sig = headers
            .get("x-slack-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        super::signature::verify_signature(slack_provider.signing_secret(), timestamp, &body, sig)?;

        let challenge = payload
            .get("challenge")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Ok((StatusCode::OK, challenge.to_string()).into_response());
    }

    let source_id = payload
        .pointer("/event/channel")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if source_id.is_empty() {
        let event_subtype = payload
            .pointer("/event/type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        warn!(
            event_type,
            event_subtype, "Slack webhook has no channel ID in event payload, ignoring"
        );
        return Ok(StatusCode::OK.into_response());
    }

    let team_id = payload
        .get("team_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    let Some((connection, rule)) = crate::channel::binding_runtime::resolve_chat_provider_event(
        &state, "slack", team_id, source_id,
    )
    .await?
    else {
        info!(
            source_id,
            team_id, "Slack message ignored: no task trigger for this channel"
        );
        return Ok(StatusCode::OK.into_response());
    };
    let config = crate::channel::binding_runtime::trigger_context(&rule, "slack");

    let thread_ts = payload.pointer("/event/thread_ts").and_then(|v| v.as_str());
    let is_tracked_thread = if let Some(tts) = thread_ts {
        let destination = slack_provider.binding_destination_from_existing_thread(
            source_id,
            tts,
            &serde_json::json!({
                "channel": source_id,
                "thread_ts": tts,
                "team_id": team_id,
            }),
            None,
        )?;
        PgTaskBindingRepository(state.db.clone())
            .find_by_subject(
                slack_provider.provider_key(),
                destination.subject_kind,
                &destination.subject_key,
            )
            .await
            .map(|rows| !rows.is_empty())
            .unwrap_or(false)
    } else {
        false
    };

    let result = slack_provider
        .parse_event(&headers, &body, &config, &state.db, is_tracked_thread)
        .await?;

    match result {
        super::provider::SlackWebhookResult::Challenge(challenge) => {
            Ok((StatusCode::OK, challenge).into_response())
        }
        super::provider::SlackWebhookResult::Message(msg) => {
            let msg = *msg;
            let destination = match slack_provider.binding_destination_from_existing_thread(
                &msg.source_id,
                &msg.message_thread_id,
                &msg.thread_properties,
                None,
            ) {
                Ok(destination) => destination,
                Err(e) => {
                    warn!(error = %e, "Failed to build Slack binding destination");
                    return Ok(StatusCode::OK.into_response());
                }
            };
            let state_clone = state.clone();
            let connection = connection.clone();
            let rule = rule.clone();
            tokio::spawn(async move {
                if let Err(e) = ProviderEventDeliveryService::new(&state_clone)
                    .deliver(
                        &connection,
                        &rule,
                        msg,
                        None,
                        "slack",
                        destination.subject_kind,
                        &destination.subject_key,
                        destination.destination_config,
                    )
                    .await
                {
                    warn!(error = %e, "Failed to process Slack inbound message");
                }
            });
            Ok(StatusCode::OK.into_response())
        }
        super::provider::SlackWebhookResult::Ignore => {
            debug!(source_id, "Slack event ignored by provider");
            Ok(StatusCode::OK.into_response())
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/integrations/slack/slash",
    tag = "integrations",
    request_body(content = String),
    responses((status = 200, description = "Command acknowledged"))
)]
pub async fn slack_slash_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ChannelError> {
    let providers = state
        .channel_providers
        .as_ref()
        .ok_or_else(|| ChannelError::ProviderNotConfigured("slack".into()))?;

    let slack = providers
        .get(ChannelType::Slack)
        .ok_or_else(|| ChannelError::ProviderNotConfigured("slack".into()))?;

    let slack_provider = slack
        .as_any()
        .downcast_ref::<super::provider::SlackProvider>()
        .ok_or_else(|| ChannelError::Internal("Provider type mismatch".into()))?;

    let timestamp = headers
        .get("x-slack-request-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let sig = headers
        .get("x-slack-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    super::signature::verify_signature(slack_provider.signing_secret(), timestamp, &body, sig)?;

    let payload: super::command::SlackSlashCommandPayload = serde_urlencoded::from_bytes(&body)
        .map_err(|e| ChannelError::BadRequest(format!("Invalid form data: {e}")))?;

    info!(
        command = %payload.command,
        channel_id = %payload.channel_id,
        user_id = %payload.user_id,
        text = %payload.text,
        "Slack slash command received"
    );

    let parsed = payload.parse_command(&["moat"]);
    let Some(parsed) = parsed else {
        return Ok(Json(serde_json::json!({
            "response_type": "ephemeral",
            "text": "Usage: /moat <command> [context]\nExample: /moat commit fix auth bug"
        })));
    };

    let command = state
        .commands
        .find_by_name(&parsed.command_name)
        .await
        .map_err(|e| ChannelError::Internal(format!("Command lookup failed: {e}")))?;

    let Some(command) = command else {
        return Ok(Json(serde_json::json!({
            "response_type": "ephemeral",
            "text": format!("Unknown command: `{}`. Use a valid system command.", parsed.command_name)
        })));
    };

    if command.active == Some(false) {
        return Ok(Json(serde_json::json!({
            "response_type": "ephemeral",
            "text": format!("Command `{}` is currently disabled.", parsed.command_name)
        })));
    }

    let command_info = crate::channel::inbound::CommandInfo {
        name: command.name.clone(),
        resume: !command.will_not_resume.unwrap_or(false),
        agent_mode: command.agent_mode.clone(),
    };

    let state_clone = state.clone();
    let slack_arc = std::sync::Arc::clone(slack);
    let channel_id = payload.channel_id.clone();
    let team_id = payload.team_id.clone();
    let slack_user_id = payload.user_id.clone();
    let user_text = parsed.user_text.clone();
    let command_name_display = parsed.command_name.clone();
    let command_name_ack = command_name_display.clone();

    tokio::spawn(async move {
        if let Err(e) = process_slash_command_background(
            &state_clone,
            &slack_arc,
            SlashCommandBackgroundInput {
                channel_id: &channel_id,
                team_id: &team_id,
                slack_user_id: &slack_user_id,
                command_name: &command_name_display,
                user_text: &user_text,
                command_info,
            },
        )
        .await
        {
            warn!(
                error = %e,
                channel = %channel_id,
                command = %command_name_display,
                "Failed to process slash command"
            );
            if let Some(slack_p) = slack_arc
                .as_any()
                .downcast_ref::<super::provider::SlackProvider>()
            {
                let _ = slack_p
                    .post_message(
                        &channel_id,
                        &format!("Failed to process /{command_name_display}: {e}"),
                        None,
                    )
                    .await;
            }
        }
    });

    Ok(Json(serde_json::json!({
        "response_type": "ephemeral",
        "text": format!("Processing /{command_name_ack}...")
    })))
}

struct SlashCommandBackgroundInput<'a> {
    channel_id: &'a str,
    team_id: &'a str,
    slack_user_id: &'a str,
    command_name: &'a str,
    user_text: &'a str,
    command_info: crate::channel::inbound::CommandInfo,
}

async fn process_slash_command_background(
    state: &AppState,
    slack_arc: &std::sync::Arc<dyn crate::channel::provider::ChannelProvider>,
    input: SlashCommandBackgroundInput<'_>,
) -> Result<(), ChannelError> {
    use crate::channel::models::InboundMessage;

    let slack_provider = slack_arc
        .as_any()
        .downcast_ref::<super::provider::SlackProvider>()
        .ok_or_else(|| ChannelError::Internal("Provider type mismatch".into()))?;

    let (connection, rule) = crate::channel::binding_runtime::resolve_chat_provider_event(
        state,
        "slack",
        "default",
        input.channel_id,
    )
    .await?
    .ok_or_else(|| {
        ChannelError::ConfigNotFound(format!("No Slack rule for channel {}", input.channel_id))
    })?;
    if !rule.active {
        return Err(ChannelError::BadRequest(
            "Slack task trigger is inactive".into(),
        ));
    }

    let user_id: String = slack_arc
        .resolve_user(input.slack_user_id, &state.db)
        .await?
        .ok_or_else(|| {
            ChannelError::BadRequest(format!(
                "Slack user {} not mapped to system user",
                input.slack_user_id
            ))
        })?;

    let display_text = if input.user_text.is_empty() {
        format!("/{}", input.command_name)
    } else {
        format!("/{} {}", input.command_name, input.user_text)
    };

    let ts = slack_provider
        .post_message(input.channel_id, &display_text, None)
        .await?
        .ok_or_else(|| ChannelError::Internal("Slack did not return message ts".into()))?;

    let thread_properties = serde_json::json!({
        "channel": input.channel_id,
        "ts": ts,
        "thread_ts": ts,
        "team_id": input.team_id,
    });

    let prefixed_ext_id = crate::channel::slack::external_id::message_ts(&ts);
    let msg = InboundMessage {
        provider: ChannelType::Slack,
        source_id: input.channel_id.to_string(),
        thread_id: None,
        message_thread_id: ts,
        user_id,
        text: input.user_text.to_string(),
        file_ids: vec![],
        thread_properties,
        provider_context: serde_json::json!({
            "user_id": input.slack_user_id,
            "is_bot_mentioned": true,
            "slash_command": true,
        }),
        channel_external_id: Some(prefixed_ext_id),
    };

    let destination = slack_arc.binding_destination_from_existing_thread(
        &msg.source_id,
        &msg.message_thread_id,
        &msg.thread_properties,
        None,
    )?;
    ProviderEventDeliveryService::new(state)
        .deliver(
            &connection,
            &rule,
            msg,
            Some(input.command_info),
            "slack",
            destination.subject_kind,
            &destination.subject_key,
            destination.destination_config,
        )
        .await?;

    Ok(())
}
