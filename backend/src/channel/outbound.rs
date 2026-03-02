//! Outbound channel notifications — event-driven replies to channel threads.

use moatless_events::{AgentSubType, TaskTurnSubType};
use tracing::{debug, info, warn};

use crate::event_handler::models::{AgentEvent, EventKey};
use crate::state::AppState;

use super::models::{ChannelConfig, ChannelThread, MessageStatus, OutboundReply};
use super::provider::{ChannelProvider, ChannelProviders};

/// Handle a channel notification triggered by an agent event.
///
/// Called from `event_handler/consumer.rs` after standard event processing.
/// Looks up channel threads for the task and sends replies/status updates.
///
/// Supports cross-channel routing: when a config has `response_config_id`,
/// the reply is sent via the response config's provider instead of the
/// originating config's provider.
pub async fn handle_channel_notification(
    state: &AppState,
    providers: &ChannelProviders,
    event: &AgentEvent,
) {
    // Find threads for this task
    let threads = match state.channel_threads.find_by_task_id(&event.task_id).await {
        Ok(t) => t,
        Err(e) => {
            debug!(task_id = %event.task_id, error = %e, "Failed to look up channel threads");
            return;
        }
    };

    if threads.is_empty() {
        return; // Task not from a channel
    }

    let key = event.event_key();

    for thread in &threads {
        // Load config for this thread
        let config = match state.channel_configs.find_by_id(&thread.config_id).await {
            Ok(Some(c)) => c,
            _ => continue,
        };

        let task_url = build_task_url(state, &event.task_id);

        // Resolve outbound target: use response_config_id if set
        let (out_provider, out_config, out_thread) =
            match resolve_outbound_target(state, providers, &config, thread, &event.task_id).await
            {
                Some(target) => target,
                None => continue,
            };

        match &key {
            EventKey::TaskTurn(TaskTurnSubType::Completed)
            | EventKey::Agent(AgentSubType::Completed) => {
                let result_text = event
                    .payload
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Task completed");

                let reply = OutboundReply {
                    text: truncate_text(result_text, 2000),
                    task_url: Some(task_url),
                    is_error: false,
                };

                if let Err(e) = out_provider.send_reply(&out_thread, &out_config, &reply).await {
                    warn!(task_id = %event.task_id, error = %e, "Failed to send channel reply");
                }
                if let Err(e) = out_provider
                    .set_status(&out_thread, &out_config, MessageStatus::Completed)
                    .await
                {
                    warn!(task_id = %event.task_id, error = %e, "Failed to set completed status");
                }
            }

            EventKey::Agent(AgentSubType::Error)
            | EventKey::TaskTurn(TaskTurnSubType::Error) => {
                let error_msg = event
                    .payload
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("An error occurred");

                let reply = OutboundReply {
                    text: format!("Error: {}", truncate_text(error_msg, 1900)),
                    task_url: Some(task_url),
                    is_error: true,
                };

                if let Err(e) = out_provider.send_reply(&out_thread, &out_config, &reply).await {
                    warn!(task_id = %event.task_id, error = %e, "Failed to send error reply");
                }
                if let Err(e) = out_provider
                    .set_status(&out_thread, &out_config, MessageStatus::Error)
                    .await
                {
                    warn!(task_id = %event.task_id, error = %e, "Failed to set error status");
                }
            }

            _ => {
                debug!(
                    task_id = %event.task_id,
                    event_type = %event.event_type,
                    sub_type = %event.sub_type,
                    "Event type not handled for channel notifications"
                );
            }
        }
    }
}

/// Resolve the outbound target: provider, config, and thread.
///
/// If the originating config has `response_config_id`, load the response config
/// and use its provider for outbound. Create or reuse a thread under the response config.
async fn resolve_outbound_target<'a>(
    state: &AppState,
    providers: &'a ChannelProviders,
    config: &ChannelConfig,
    thread: &ChannelThread,
    task_id: &str,
) -> Option<(&'a dyn ChannelProvider, ChannelConfig, ChannelThread)> {
    if let Some(ref response_config_id) = config.response_config_id {
        // Cross-channel: load response config
        let response_config = match state
            .channel_configs
            .find_by_id(response_config_id)
            .await
        {
            Ok(Some(c)) => c,
            Ok(None) => {
                warn!(
                    config_id = %config.id,
                    response_config_id = %response_config_id,
                    "Response config not found, falling back to originating provider"
                );
                return resolve_direct(providers, config, thread);
            }
            Err(e) => {
                warn!(error = %e, "Failed to load response config, falling back");
                return resolve_direct(providers, config, thread);
            }
        };

        let Some(response_provider) = providers.get_by_name(&response_config.provider) else {
            warn!(
                provider = %response_config.provider,
                "Response provider not configured, falling back"
            );
            return resolve_direct(providers, config, thread);
        };
        let response_provider = response_provider.as_ref();

        // Find or create thread under response config
        let response_thread = match state
            .channel_threads
            .find_by_task_id_and_config(task_id, response_config_id)
            .await
        {
            Ok(Some(t)) => t,
            Ok(None) => {
                // Create a new thread under the response config
                match state
                    .channel_threads
                    .insert(
                        response_config_id,
                        task_id,
                        &thread.external_thread_id,
                        &serde_json::json!({"cross_channel_from": config.id}),
                    )
                    .await
                {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(error = %e, "Failed to create response thread, falling back");
                        return resolve_direct(providers, config, thread);
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to look up response thread, falling back");
                return resolve_direct(providers, config, thread);
            }
        };

        info!(
            config_id = %config.id,
            response_config_id = %response_config_id,
            response_provider = %response_config.provider,
            "Cross-channel routing: outbound via response config"
        );

        Some((response_provider, response_config, response_thread))
    } else {
        resolve_direct(providers, config, thread)
    }
}

/// Resolve direct outbound (no cross-channel routing).
fn resolve_direct<'a>(
    providers: &'a ChannelProviders,
    config: &ChannelConfig,
    thread: &ChannelThread,
) -> Option<(&'a dyn ChannelProvider, ChannelConfig, ChannelThread)> {
    let provider = providers.get_by_name(&config.provider)?;
    Some((provider.as_ref(), config.clone(), thread.clone()))
}

/// Build the task URL for linking in channel replies.
fn build_task_url(state: &AppState, task_id: &str) -> String {
    format!("{}/tasks/{}", state.config.auth.frontend_url, task_id)
}

/// Truncate text to a max length, appending "..." if truncated.
fn truncate_text(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        format!("{}...", &text[..max.saturating_sub(3)])
    }
}
