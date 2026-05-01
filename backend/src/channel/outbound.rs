//! Outbound channel notifications — event-driven replies and explicit sends.

use std::collections::HashSet;

use moatless_events::{AgentSubType, TaskTurnSubType};
use tracing::{debug, info, warn};

use crate::event_handler::models::{AgentEvent, EventKey};
use crate::state::AppState;

use super::error::ChannelError;
use super::models::{ChannelConfig, ChannelThread, OutboundFile, OutboundReply};
use super::provider::{ChannelProvider, ChannelProviders};

/// Resolved outbound target for a channel notification.
pub(crate) enum OutboundTarget<'a> {
    /// Thread exists — use `send_reply`.
    Reply {
        provider: &'a dyn ChannelProvider,
        config: ChannelConfig,
        thread: ChannelThread,
    },
    /// No thread yet — use `send_message` to post a top-level message.
    /// If `pending_thread_id` is set, update the existing pending thread
    /// instead of inserting a new one.
    NewMessage {
        provider: &'a dyn ChannelProvider,
        config: ChannelConfig,
        originating_config_id: String,
        task_id: String,
        pending_thread_id: Option<i32>,
    },
}

/// Send a message to channel(s) for a task (explicit send from agent/CLI).
///
/// If `config_id` is `Some`, sends only to that channel; otherwise sends to all.
/// When `file_ids` is non-empty, files are resolved from S3 and included in the reply.
/// Providers that don't support files will receive an error.
/// Returns `(sent_count, errors)`.
pub async fn send_to_channels(
    state: &AppState,
    providers: &ChannelProviders,
    task_id: &str,
    text: &str,
    file_ids: &[String],
    config_id: Option<&str>,
) -> Result<(u32, Vec<String>), ChannelError> {
    let threads = if let Some(cid) = config_id {
        match state
            .channel_threads
            .find_by_task_id_and_config(task_id, cid)
            .await?
        {
            Some(t) => vec![t],
            None => {
                return Err(ChannelError::ConfigNotFound(format!(
                    "No channel thread for task {task_id} and config {cid}"
                )));
            }
        }
    } else {
        let t = state.channel_threads.find_by_task_id(task_id).await?;
        if t.is_empty() {
            return Err(ChannelError::ConfigNotFound(format!(
                "No channel threads for task {task_id}"
            )));
        }
        t
    };

    let files = resolve_files(state, file_ids).await;

    let mut sent_count = 0u32;
    let mut errors = Vec::new();
    let mut sent_config_ids: HashSet<String> = HashSet::new();

    for thread in &threads {
        let config = match state.channel_configs.find_by_id(&thread.config_id).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                errors.push(format!("Config {} not found", thread.config_id));
                continue;
            }
            Err(e) => {
                errors.push(format!("Failed to load config {}: {e}", thread.config_id));
                continue;
            }
        };

        // Deduplicate by effective target config: if a config has response_config_id,
        // that's the actual target; otherwise the config itself is the target.
        // This prevents duplicate sends when both a cross-channel thread (e.g. cron)
        // and its response config's direct thread exist for the same task.
        let effective_config_id = config.response_config_id.as_deref().unwrap_or(&config.id);
        if !sent_config_ids.insert(effective_config_id.to_string()) {
            debug!(
                config_id = %config.id,
                effective_config_id = %effective_config_id,
                "Skipping duplicate send to already-targeted config"
            );
            continue;
        }

        let target = if let Some(t) =
            resolve_outbound_target(state, providers, &config, thread, task_id).await
        {
            t
        } else {
            errors.push(format!("No provider for config {}", config.id));
            continue;
        };

        // Reject if files are attached but the provider doesn't support them.
        if !files.is_empty() && !target_supports_files(&target) {
            let provider_name = target_provider_type(&target);
            errors.push(format!(
                "Provider {provider_name:?} does not support file attachments"
            ));
            continue;
        }

        let reply = OutboundReply {
            text: truncate_text(text, target_max_message_length(&target)),
            task_url: None,
            is_error: false,
            files: files.clone(),
        };

        match send_outbound_result(state, task_id, target, &reply).await {
            Ok(()) => sent_count += 1,
            Err(e) => errors.push(e),
        }
    }

    Ok((sent_count, errors))
}

/// Send a message directly to a channel config.
///
/// The config must have `allow_direct_send = true` and the provider must support
/// `send_message()`.
///
/// When `task_id` is provided, the thread is tracked: the first send creates a
/// top-level message and persists a `channel_thread` record; subsequent sends
/// from the same task reply in that thread. This allows `moat reply` and
/// `moat channels send` to continue the conversation.
///
/// When `task_id` is `None`, the send is fire-and-forget with no persistent state.
///
/// Returns the provider-specific external message ID.
pub async fn direct_send_to_channel(
    state: &AppState,
    providers: &ChannelProviders,
    config_id: &str,
    task_id: Option<&str>,
    text: &str,
    file_ids: &[String],
) -> Result<Option<String>, ChannelError> {
    let config = state
        .channel_configs
        .find_by_id(config_id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(config_id.to_string()))?;

    if !config.allow_direct_send {
        return Err(ChannelError::BadRequest(format!(
            "Channel config {config_id} does not allow direct send"
        )));
    }

    if !config.active {
        return Err(ChannelError::BadRequest(format!(
            "Channel config {config_id} is not active"
        )));
    }

    let provider = providers
        .get_by_name(&config.provider)
        .ok_or_else(|| ChannelError::ProviderNotConfigured(config.provider.clone()))?;

    let files = resolve_files(state, file_ids).await;

    if !files.is_empty() && !provider.supports_files() {
        return Err(ChannelError::BadRequest(format!(
            "Provider {:?} does not support file attachments",
            provider.channel_type()
        )));
    }

    let reply = OutboundReply {
        text: truncate_text(text, provider.max_message_length()),
        task_url: None,
        is_error: false,
        files,
    };

    // If we have a task_id, check for an existing thread to reply into.
    if let Some(tid) = task_id {
        if let Some(thread) = state
            .channel_threads
            .find_by_task_id_and_config(tid, config_id)
            .await?
        {
            // Existing thread — reply into it.
            let external_id = provider
                .send_reply(&thread, &config, &reply)
                .await
                .map_err(|e| {
                    ChannelError::ProviderApi(format!(
                        "Failed to send reply via {}: {e}",
                        config.provider
                    ))
                })?;

            // Advance cursor for context fetching.
            if let Some(ref ext_id) = external_id {
                if let Err(e) = state
                    .channel_threads
                    .touch(
                        thread.id,
                        Some(&serde_json::json!({"last_channel_external_id": ext_id})),
                    )
                    .await
                {
                    warn!(task_id = %tid, error = %e, "Failed to update last_channel_external_id");
                }
            }

            info!(
                config_id = %config_id,
                task_id = %tid,
                "Direct reply sent to existing channel thread"
            );
            return Ok(external_id);
        }
    }

    // No existing thread (or no task_id) — send a new top-level message.
    let thread_info = provider.send_message(&config, &reply).await.map_err(|e| {
        ChannelError::ProviderApi(format!(
            "Failed to send message via {}: {e}",
            config.provider
        ))
    })?;

    // Persist the thread so future sends from this task reply in it.
    if let Some(tid) = task_id {
        if let Err(e) = state
            .channel_threads
            .insert(
                config_id,
                tid,
                &thread_info.external_thread_id,
                &thread_info.properties,
            )
            .await
        {
            warn!(
                task_id = %tid,
                config_id = %config_id,
                error = %e,
                "Failed to persist channel thread after direct send"
            );
        } else {
            info!(
                config_id = %config_id,
                task_id = %tid,
                "Direct send: new channel thread created"
            );
        }
    } else {
        info!(
            config_id = %config_id,
            "Direct message sent (fire-and-forget, no task)"
        );
    }

    Ok(Some(thread_info.external_thread_id))
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

    let mut sent_config_ids: HashSet<String> = HashSet::new();

    for thread in &threads {
        // Load config for this thread
        let config = match state.channel_configs.find_by_id(&thread.config_id).await {
            Ok(Some(c)) => c,
            _ => continue,
        };

        // Deduplicate by effective target config (see send_to_channels).
        let effective_config_id = config.response_config_id.as_deref().unwrap_or(&config.id);
        if !sent_config_ids.insert(effective_config_id.to_string()) {
            debug!(
                task_id = %event.task_id,
                config_id = %config.id,
                effective_config_id = %effective_config_id,
                "Skipping duplicate notification to already-targeted config"
            );
            continue;
        }

        let task_url = build_task_url(state, &event.task_id);

        // Resolve outbound target: use response_config_id if set
        let target = match resolve_outbound_target(
            state,
            providers,
            &config,
            thread,
            &event.task_id,
        )
        .await
        {
            Some(t) => t,
            None => continue,
        };

        // Extract message_id from payload (injected by agent_handlers for outbound tracking)
        let message_id = event.payload.get("message_id").and_then(|v| v.as_str());

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
                    files: vec![],
                };

                send_outbound(state, &event.task_id, target, &reply, message_id).await;
            }

            EventKey::Agent(AgentSubType::Error) | EventKey::TaskTurn(TaskTurnSubType::Error) => {
                let error_msg = event
                    .payload
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("An error occurred");

                let reply = OutboundReply {
                    text: format!("Error: {}", truncate_text(error_msg, 1900)),
                    task_url: if is_first_reply { Some(task_url) } else { None },
                    is_error: true,
                    files: vec![],
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
                        if let Err(e) = state
                            .messages
                            .set_channel_external_id(msg_id, &external_id)
                            .await
                        {
                            warn!(task_id = %task_id, error = %e, "Failed to store outbound external ID");
                        }
                    }
                    // Advance cursor so future context fetches skip this reply
                    if let Err(e) = state
                        .channel_threads
                        .touch(
                            thread.id,
                            Some(&serde_json::json!({"last_channel_external_id": external_id})),
                        )
                        .await
                    {
                        warn!(task_id = %task_id, error = %e, "Failed to update last_channel_external_id on thread");
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
            pending_thread_id,
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

            // Persist the thread so future replies thread under this message.
            // If a pending thread exists (created during inbound for non-replyable
            // providers like cron), update it with the real thread info.
            if let Some(thread_id) = pending_thread_id {
                match state
                    .channel_threads
                    .update_thread_info(
                        thread_id,
                        &thread_info.external_thread_id,
                        &thread_info.properties,
                    )
                    .await
                {
                    Ok(()) => {
                        info!(
                            task_id = %task_id,
                            config_id = %config.id,
                            originating_config_id = %originating_config_id,
                            "Pending thread updated with real channel info"
                        );
                    }
                    Err(e) => {
                        warn!(
                            task_id = %task_id,
                            error = %e,
                            "Failed to update pending thread after send_message"
                        );
                    }
                }
            } else {
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
                            "Channel message sent and thread created"
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
}

/// Send an outbound reply and return `Ok(())` on success or an error string on failure.
///
/// Similar to [`send_outbound`] but returns errors instead of logging them.
async fn send_outbound_result(
    state: &AppState,
    task_id: &str,
    target: OutboundTarget<'_>,
    reply: &OutboundReply,
) -> Result<(), String> {
    match target {
        OutboundTarget::Reply {
            provider,
            config,
            thread,
        } => {
            let external_id = provider
                .send_reply(&thread, &config, reply)
                .await
                .map_err(|e| format!("Failed to send reply via {}: {e}", config.provider))?;

            // Advance cursor so future context fetches skip this reply
            if let Some(ref ext_id) = external_id {
                if let Err(e) = state
                    .channel_threads
                    .touch(
                        thread.id,
                        Some(&serde_json::json!({"last_channel_external_id": ext_id})),
                    )
                    .await
                {
                    warn!(task_id = %task_id, error = %e, "Failed to update last_channel_external_id on thread");
                }
            }

            Ok(())
        }
        OutboundTarget::NewMessage {
            provider,
            config,
            originating_config_id,
            task_id: ref tid,
            pending_thread_id,
        } => {
            let thread_info = provider
                .send_message(&config, reply)
                .await
                .map_err(|e| format!("Failed to send message via {}: {e}", config.provider))?;

            // Persist the thread so future replies thread under this message
            if let Some(thread_id) = pending_thread_id {
                if let Err(e) = state
                    .channel_threads
                    .update_thread_info(
                        thread_id,
                        &thread_info.external_thread_id,
                        &thread_info.properties,
                    )
                    .await
                {
                    warn!(
                        task_id = %task_id,
                        error = %e,
                        "Failed to update pending thread after send_message"
                    );
                } else {
                    info!(
                        task_id = %task_id,
                        config_id = %config.id,
                        originating_config_id = %originating_config_id,
                        "Pending thread updated with real channel info"
                    );
                }
            } else if let Err(e) = state
                .channel_threads
                .insert(
                    &config.id,
                    tid,
                    &thread_info.external_thread_id,
                    &thread_info.properties,
                )
                .await
            {
                warn!(
                    task_id = %task_id,
                    error = %e,
                    "Failed to persist channel thread after send_message"
                );
            } else {
                info!(
                    task_id = %task_id,
                    config_id = %config.id,
                    originating_config_id = %originating_config_id,
                    "Channel message sent and thread created"
                );
            }
            Ok(())
        }
    }
}

/// Resolve the outbound target: provider, config, and thread (or new-message intent).
///
/// If the originating config has `response_config_id`, load the response config
/// and use its provider for outbound. If a thread already exists under the response
/// config, return `Reply`; otherwise return `NewMessage`.
pub(crate) async fn resolve_outbound_target<'a>(
    state: &AppState,
    providers: &'a ChannelProviders,
    config: &ChannelConfig,
    thread: &ChannelThread,
    task_id: &str,
) -> Option<OutboundTarget<'a>> {
    if let Some(ref response_config_id) = config.response_config_id {
        // Cross-channel: load response config
        let response_config = match state.channel_configs.find_by_id(response_config_id).await {
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
                    pending_thread_id: None,
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
///
/// If the thread is pending (created during inbound for a non-replyable provider
/// like cron), returns `NewMessage` so the first outbound send creates a real
/// channel message and updates the thread with real provider info.
fn resolve_direct<'a>(
    providers: &'a ChannelProviders,
    config: &ChannelConfig,
    thread: &ChannelThread,
) -> Option<OutboundTarget<'a>> {
    let provider = providers.get_by_name(&config.provider)?;

    let is_pending = thread
        .properties
        .get("pending")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    if is_pending {
        Some(OutboundTarget::NewMessage {
            provider: provider.as_ref(),
            config: config.clone(),
            originating_config_id: config.id.clone(),
            task_id: thread.task_id.clone(),
            pending_thread_id: Some(thread.id),
        })
    } else {
        Some(OutboundTarget::Reply {
            provider: provider.as_ref(),
            config: config.clone(),
            thread: thread.clone(),
        })
    }
}

/// Build the task URL for linking in channel replies.
pub(crate) fn build_task_url(state: &AppState, task_id: &str) -> String {
    format!("{}/tasks/{}", state.config.auth.frontend_url, task_id)
}

/// Truncate text to a max length, appending "..." if truncated.
pub(crate) fn truncate_text(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        format!("{}...", &text[..max.saturating_sub(3)])
    }
}

// ---------------------------------------------------------------------------
// File attachment helpers
// ---------------------------------------------------------------------------

/// Resolve file IDs to [`OutboundFile`] structs by loading metadata + data from S3.
///
/// Failures for individual files are logged as warnings and skipped.
async fn resolve_files(state: &AppState, file_ids: &[String]) -> Vec<OutboundFile> {
    if file_ids.is_empty() {
        return vec![];
    }

    let Some(s3_client) = state.s3.as_ref() else {
        warn!("S3 not configured, cannot resolve file attachments");
        return vec![];
    };
    let Some(bucket) = state.config.s3.bucket_name.as_ref() else {
        warn!("S3 bucket not configured, cannot resolve file attachments");
        return vec![];
    };

    let mut files = Vec::with_capacity(file_ids.len());
    for file_id in file_ids {
        let meta = match state.files.find_by_id(file_id).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                warn!(file_id = %file_id, "File metadata not found for outbound attachment");
                continue;
            }
            Err(e) => {
                warn!(file_id = %file_id, error = %e, "Failed to load file metadata");
                continue;
            }
        };

        let s3_key = format!("{}/{}", state.config.s3.key_prefix, file_id);
        match crate::file_storage::s3::get_object(s3_client, bucket, &s3_key).await {
            Ok((data, _content_type)) => {
                files.push(OutboundFile {
                    filename: meta.filename,
                    mime_type: meta.mime_type,
                    data,
                });
            }
            Err(e) => {
                warn!(file_id = %file_id, error = %e, "Failed to download file from S3");
            }
        }
    }

    files
}

/// Check whether the resolved outbound target's provider supports file attachments.
fn target_supports_files(target: &OutboundTarget<'_>) -> bool {
    match target {
        OutboundTarget::Reply { provider, .. } | OutboundTarget::NewMessage { provider, .. } => {
            provider.supports_files()
        }
    }
}

/// Maximum reply text length supported by the resolved target's provider.
fn target_max_message_length(target: &OutboundTarget<'_>) -> usize {
    match target {
        OutboundTarget::Reply { provider, .. } | OutboundTarget::NewMessage { provider, .. } => {
            provider.max_message_length()
        }
    }
}

/// Get the channel type of the resolved outbound target's provider (for error messages).
fn target_provider_type(target: &OutboundTarget<'_>) -> super::provider::ChannelType {
    match target {
        OutboundTarget::Reply { provider, .. } | OutboundTarget::NewMessage { provider, .. } => {
            provider.channel_type()
        }
    }
}
