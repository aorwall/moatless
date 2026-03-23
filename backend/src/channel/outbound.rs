//! Outbound channel notifications — event-driven replies to channel threads.

use moatless_events::{AgentSubType, TaskTurnSubType};
use tracing::{debug, info, warn};

use crate::event_handler::models::{AgentEvent, EventKey};
use crate::state::AppState;

use super::models::{ChannelConfig, ChannelThread, OutboundReply};
use super::provider::{ChannelProvider, ChannelProviders};

/// Resolved outbound target for a channel notification.
enum OutboundTarget<'a> {
    /// Thread exists — use `send_reply`.
    Reply {
        provider: &'a dyn ChannelProvider,
        config: ChannelConfig,
        thread: ChannelThread,
    },
    /// No thread yet (cross-channel) — use `send_message` to create one.
    NewMessage {
        provider: &'a dyn ChannelProvider,
        config: ChannelConfig,
        originating_config_id: String,
        task_id: String,
    },
}

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

    // Only include the task URL on the first completion (new task).
    let is_first_reply = state
        .messages
        .get_last_result(&event.task_id)
        .await
        .ok()
        .flatten()
        .is_none();

    for thread in &threads {
        // Load config for this thread
        let config = match state.channel_configs.find_by_id(&thread.config_id).await {
            Ok(Some(c)) => c,
            _ => continue,
        };

        let task_url = build_task_url(state, &event.task_id);

        // Resolve outbound target: use response_config_id if set
        let target =
            match resolve_outbound_target(state, providers, &config, thread, &event.task_id).await
            {
                Some(t) => t,
                None => continue,
            };

        // Extract message_id from payload (injected by agent_handlers for outbound tracking)
        let message_id = event
            .payload
            .get("message_id")
            .and_then(|v| v.as_str());

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
                    task_url: if is_first_reply { Some(task_url) } else { None },
                    is_error: false,
                };

                send_outbound(state, &event.task_id, target, &reply, message_id)
                    .await;
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
                    task_url: if is_first_reply { Some(task_url) } else { None },
                    is_error: true,
                };

                send_outbound(state, &event.task_id, target, &reply, message_id).await;
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

/// Send an outbound reply via the resolved target.
///
/// For `Reply` targets, uses `send_reply` on an existing thread.
/// For `NewMessage` targets, uses `send_message` to post a top-level message
/// and persists the thread for future replies.
///
/// If `message_id` is provided and the provider returns an external ID,
/// stores the mapping via `set_channel_external_id` for outbound tracking.
async fn send_outbound(
    state: &AppState,
    task_id: &str,
    target: OutboundTarget<'_>,
    reply: &OutboundReply,
    message_id: Option<&str>,
) {
    match target {
        OutboundTarget::Reply {
            provider,
            config,
            thread,
        } => {
            match provider.send_reply(&thread, &config, reply).await {
                Ok(Some(external_id)) => {
                    if let Some(msg_id) = message_id {
                        if let Err(e) = state.messages.set_channel_external_id(msg_id, &external_id).await {
                            warn!(task_id = %task_id, error = %e, "Failed to store outbound external ID");
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Failed to send channel reply");
                }
            }
        }
        OutboundTarget::NewMessage {
            provider,
            config,
            originating_config_id,
            task_id: ref tid,
        } => {
            let thread_info = match provider.send_message(&config, reply).await {
                Ok(info) => info,
                Err(e) => {
                    warn!(
                        task_id = %task_id,
                        config_id = %config.id,
                        error = %e,
                        "Failed to send new channel message"
                    );
                    return;
                }
            };

            // Persist the thread so future replies thread under this message
            match state
                .channel_threads
                .insert(
                    &config.id,
                    tid,
                    &thread_info.external_thread_id,
                    &thread_info.properties,
                )
                .await
            {
                Ok(_thread) => {
                    info!(
                        task_id = %task_id,
                        config_id = %config.id,
                        originating_config_id = %originating_config_id,
                        "Cross-channel message sent and thread created"
                    );
                }
                Err(e) => {
                    warn!(
                        task_id = %task_id,
                        error = %e,
                        "Failed to persist channel thread after send_message"
                    );
                }
            }
        }
    }
}

/// Resolve the outbound target: provider, config, and thread (or new-message intent).
///
/// If the originating config has `response_config_id`, load the response config
/// and use its provider for outbound. If a thread already exists under the response
/// config, return `Reply`; otherwise return `NewMessage`.
async fn resolve_outbound_target<'a>(
    state: &AppState,
    providers: &'a ChannelProviders,
    config: &ChannelConfig,
    thread: &ChannelThread,
    task_id: &str,
) -> Option<OutboundTarget<'a>> {
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

        // Check if a thread already exists under the response config
        match state
            .channel_threads
            .find_by_task_id_and_config(task_id, response_config_id)
            .await
        {
            Ok(Some(response_thread)) => {
                info!(
                    config_id = %config.id,
                    response_config_id = %response_config_id,
                    response_provider = %response_config.provider,
                    "Cross-channel routing: reply via existing response thread"
                );
                Some(OutboundTarget::Reply {
                    provider: response_provider,
                    config: response_config,
                    thread: response_thread,
                })
            }
            Ok(None) => {
                info!(
                    config_id = %config.id,
                    response_config_id = %response_config_id,
                    response_provider = %response_config.provider,
                    "Cross-channel routing: new message (no existing response thread)"
                );
                Some(OutboundTarget::NewMessage {
                    provider: response_provider,
                    config: response_config,
                    originating_config_id: config.id.clone(),
                    task_id: task_id.to_string(),
                })
            }
            Err(e) => {
                warn!(error = %e, "Failed to look up response thread, falling back");
                resolve_direct(providers, config, thread)
            }
        }
    } else {
        resolve_direct(providers, config, thread)
    }
}

/// Resolve direct outbound (no cross-channel routing).
fn resolve_direct<'a>(
    providers: &'a ChannelProviders,
    config: &ChannelConfig,
    thread: &ChannelThread,
) -> Option<OutboundTarget<'a>> {
    let provider = providers.get_by_name(&config.provider)?;
    Some(OutboundTarget::Reply {
        provider: provider.as_ref(),
        config: config.clone(),
        thread: thread.clone(),
    })
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
