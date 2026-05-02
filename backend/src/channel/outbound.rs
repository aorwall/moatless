//! Outbound channel notifications — event-driven replies and explicit sends.

use std::collections::HashSet;

use tracing::{debug, info, warn};

use crate::message::models::{
    ContentBlockType, MessageRole, MessageType, TaskMessage, TaskMessageContentBlock,
};
use crate::state::AppState;

use super::error::ChannelError;
use super::ingress_rule_dao::PgIngressRuleRepository;
use super::models::{ChannelConfig, ChannelThread, OutboundFile, OutboundReply};
use super::provider::{ChannelProvider, ChannelProviders};
use super::provider_connection_dao::PgProviderConnectionRepository;
use super::task_subscription_dao::PgTaskSubscriptionRepository;

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
        let matching: Vec<_> = state
            .channel_threads
            .find_by_task_id(task_id)
            .await?
            .into_iter()
            .filter(|thread| thread.config_id == cid)
            .collect();

        match matching.len() {
            0 => {
                return Err(ChannelError::ConfigNotFound(format!(
                    "No channel thread for task {task_id} and config {cid}"
                )));
            }
            1 => matching,
            _ => {
                return Err(ChannelError::BadRequest(format!(
                    "Task {task_id} has multiple channel threads for config {cid}; target a specific thread or send to all task channels"
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
    let mut sent_target_keys: HashSet<String> = HashSet::new();

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

        let dedup_key = outbound_target_key(&config, thread);
        if !sent_target_keys.insert(dedup_key.clone()) {
            debug!(
                config_id = %config.id,
                dedup_key = %dedup_key,
                "Skipping duplicate send to already-targeted outbound subject"
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

        // Persist a stub assistant row up-front so we have a uuid to tag with
        // the outbound `channel_external_id` once `send_reply` returns the id.
        // Also makes the reply visible in the task's message timeline (parity
        // with the agent-event-driven `send_outbound` path).
        let stub_uuid = persist_outbound_stub(state, task_id, &reply.text, &config).await;

        match send_outbound_result(state, task_id, target, &reply, stub_uuid.as_deref()).await {
            Ok(()) => sent_count += 1,
            Err(e) => errors.push(e),
        }
    }

    Ok((sent_count, errors))
}

/// Send a message to one exact channel thread for a task.
///
/// This is the precise outbound path used when a task has multiple subscriptions
/// under the same channel config, such as multiple GitHub PR subscriptions.
pub async fn send_to_channel_thread(
    state: &AppState,
    providers: &ChannelProviders,
    task_id: &str,
    thread_id: i32,
    text: &str,
    file_ids: &[String],
) -> Result<(u32, Vec<String>), ChannelError> {
    let thread = state
        .channel_threads
        .find_by_id(thread_id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(format!("No channel thread {thread_id}")))?;

    if thread.task_id != task_id {
        return Err(ChannelError::BadRequest(format!(
            "Channel thread {thread_id} does not belong to task {task_id}"
        )));
    }

    let config = state
        .channel_configs
        .find_by_id(&thread.config_id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(thread.config_id.clone()))?;

    let target = resolve_outbound_target(state, providers, &config, &thread, task_id)
        .await
        .ok_or_else(|| ChannelError::ProviderNotConfigured(config.provider.clone()))?;

    let files = resolve_files(state, file_ids).await;
    if !files.is_empty() && !target_supports_files(&target) {
        return Ok((
            0,
            vec![format!(
                "Provider {:?} does not support file attachments",
                target_provider_type(&target)
            )],
        ));
    }

    let reply = OutboundReply {
        text: truncate_text(text, target_max_message_length(&target)),
        task_url: None,
        is_error: false,
        files,
    };

    let stub_uuid = persist_outbound_stub(state, task_id, &reply.text, &config).await;

    match send_outbound_result(state, task_id, target, &reply, stub_uuid.as_deref()).await {
        Ok(()) => Ok((1, vec![])),
        Err(e) => Ok((0, vec![e])),
    }
}

fn outbound_target_key(config: &ChannelConfig, _thread: &ChannelThread) -> String {
    config
        .response_config_id
        .clone()
        .unwrap_or_else(|| config.id.clone())
}

/// Persist a visible assistant row representing an outbound channel reply.
///
/// Returns the row's uuid on success so the caller can later stamp it with
/// `channel_external_id`. Failures are logged and `None` returned — the
/// outbound send still happens, the dedup just falls back to the in-memory
/// `recently_sent` cache + future webhook delivery.
async fn persist_outbound_stub(
    state: &AppState,
    task_id: &str,
    text: &str,
    config: &ChannelConfig,
) -> Option<String> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let uuid = uuid::Uuid::new_v4().to_string();

    let turn_number = match state.messages.current_turn_number(task_id).await {
        Ok(t) => Some(t),
        Err(e) => {
            warn!(task_id, error = %e, "Failed to read turn number for outbound stub");
            None
        }
    };

    let content = serde_json::json!([{ "type": "text", "text": text }]);

    let mut msg =
        TaskMessage::new_user_message(task_id.to_string(), uuid.clone(), content, now.clone());
    msg.message_type = MessageType::System;
    msg.message_role = MessageRole::Assistant;
    msg.user_type = "assistant".to_string();
    msg.turn_number = turn_number;
    msg.channel_provider = Some(config.provider.clone());
    msg.channel_source_name = config.source_name.clone();

    let block = TaskMessageContentBlock {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: ContentBlockType::Text,
        show_in_ui: true,
        text_content: Some(text.to_string()),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    };

    match state.messages.append(&msg, &[], &[block]).await {
        Ok(_) => Some(uuid),
        Err(e) => {
            warn!(task_id, error = %e, "Failed to persist outbound stub message");
            None
        }
    }
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
            // Persist a visible stub row up-front so we can tag it with the
            // outbound `channel_external_id` once the provider returns it.
            let stub_uuid = persist_outbound_stub(state, tid, &reply.text, &config).await;

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

            // Tag the originating row + populate the recently_sent cache so
            // the inbound webhook for this same comment/message is deduped.
            if let (Some(ref ext_id), Some(ref msg_id)) = (&external_id, &stub_uuid) {
                state.recently_sent.note(tid, ext_id);
                if let Err(e) = state
                    .messages
                    .set_channel_metadata(
                        msg_id,
                        ext_id,
                        &config.provider,
                        config.source_name.as_deref(),
                    )
                    .await
                {
                    warn!(task_id = %tid, error = %e, "Failed to tag outbound stub with channel_external_id");
                }
            } else if let Some(ref ext_id) = external_id {
                state.recently_sent.note(tid, ext_id);
            }

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

/// Send a message directly through an ingress rule.
///
/// This is the rules-based replacement for direct sends to Slack/Telegram
/// channel configs. When `task_id` is provided, a subscription is created so
/// future replies can target the external thread via `moat reply`.
pub async fn direct_send_to_rule(
    state: &AppState,
    providers: &ChannelProviders,
    rule_id: &str,
    task_id: Option<&str>,
    text: &str,
    file_ids: &[String],
) -> Result<Option<String>, ChannelError> {
    let rule = PgIngressRuleRepository(state.db.clone())
        .find_by_id(rule_id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(rule_id.to_string()))?;

    if !rule.allow_direct_send {
        return Err(ChannelError::BadRequest(format!(
            "Ingress rule {rule_id} does not allow direct send"
        )));
    }
    if !rule.active {
        return Err(ChannelError::BadRequest(format!(
            "Ingress rule {rule_id} is not active"
        )));
    }

    let connection = PgProviderConnectionRepository(state.db.clone())
        .find_by_id(&rule.connection_id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(rule.connection_id.clone()))?;

    let provider_label = subscription_provider_label(&rule.provider);
    let provider = providers
        .get_by_name(provider_label)
        .ok_or_else(|| ChannelError::ProviderNotConfigured(rule.provider.clone()))?;

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
    let config = super::subscription::synthetic_config(&rule, provider_label);
    let thread_info = provider.send_message(&config, &reply).await.map_err(|e| {
        ChannelError::ProviderApi(format!("Failed to send message via {}: {e}", rule.provider))
    })?;

    if let Some(tid) = task_id {
        let (subject_kind, subject_key, reply_target) = match rule.provider.as_str() {
            "slack" => {
                let channel = thread_info
                    .properties
                    .get("channel")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&rule.scope_matcher);
                let thread_ts = thread_info
                    .properties
                    .get("thread_ts")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&thread_info.external_thread_id);
                (
                    "slack_thread",
                    format!("default:{channel}:{thread_ts}"),
                    thread_info.properties,
                )
            }
            "telegram" => {
                let chat_id = thread_info
                    .properties
                    .get("chat_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&rule.scope_matcher);
                (
                    "telegram_thread",
                    format!("{chat_id}:thread:{}", thread_info.external_thread_id),
                    thread_info.properties,
                )
            }
            other => {
                return Err(ChannelError::BadRequest(format!(
                    "Provider {other} does not support rules-based direct send"
                )));
            }
        };

        super::subscription::upsert_subscription(
            state,
            &connection,
            Some(&rule),
            tid,
            &rule.provider,
            subject_kind,
            &subject_key,
            reply_target,
            super::subscription::CREATED_REASON_CONNECTED_THREAD,
            Some(thread_info.external_thread_id.clone()),
        )
        .await?;
    }

    Ok(Some(thread_info.external_thread_id))
}

/// Send an outbound reply and return `Ok(())` on success or an error string on failure.
///
/// Similar to [`send_outbound`] but returns errors instead of logging them.
///
/// When `message_id` is provided and `send_reply` returns an external id, the
/// originating message row is tagged with that id so the inbound webhook for
/// the same comment is deduped per-task by `send_message_flow`'s DB check.
async fn send_outbound_result(
    state: &AppState,
    task_id: &str,
    target: OutboundTarget<'_>,
    reply: &OutboundReply,
    message_id: Option<&str>,
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

            // Tag originating row + populate recently_sent cache so the
            // inbound webhook for this same comment is deduped per-task.
            if let Some(ref ext_id) = external_id {
                state.recently_sent.note(task_id, ext_id);
                if let Some(msg_id) = message_id {
                    if let Err(e) = state
                        .messages
                        .set_channel_metadata(
                            msg_id,
                            ext_id,
                            &config.provider,
                            config.source_name.as_deref(),
                        )
                        .await
                    {
                        warn!(task_id = %task_id, error = %e, "Failed to tag outbound stub with channel_external_id");
                    }
                }
            }

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

// ---------------------------------------------------------------------------
// Subscription-based outbound (used by GitHub PR)
// ---------------------------------------------------------------------------

/// Send a reply to the channel referenced by a `TaskSubscription`.
///
/// Loads the subscription, its `ProviderConnection`, and its optional
/// `IngressRule`, then calls `send_reply_for_subscription` on the matching
/// provider. Persists a visible assistant stub up front, tags it with the
/// returned external id, populates the recently_sent race-window cache, and
/// advances the subscription cursor.
///
/// Returns `(sent_count, errors)` mirroring [`send_to_channels`].
pub async fn send_to_subscription(
    state: &AppState,
    providers: &ChannelProviders,
    task_id: &str,
    subscription_id: &str,
    text: &str,
    file_ids: &[String],
) -> Result<(u32, Vec<String>), ChannelError> {
    let subscription_repo = PgTaskSubscriptionRepository(state.db.clone());
    let subscription = subscription_repo
        .find_by_id(subscription_id)
        .await?
        .ok_or_else(|| {
            ChannelError::ConfigNotFound(format!("No task subscription {subscription_id}"))
        })?;

    if subscription.task_id != task_id {
        return Err(ChannelError::BadRequest(format!(
            "Subscription {subscription_id} does not belong to task {task_id}"
        )));
    }

    let connection = PgProviderConnectionRepository(state.db.clone())
        .find_by_id(&subscription.connection_id)
        .await?
        .ok_or_else(|| {
            ChannelError::ConfigNotFound(format!(
                "Provider connection {} not found",
                subscription.connection_id
            ))
        })?;

    let rule = if let Some(ref rule_id) = subscription.ingress_rule_id {
        PgIngressRuleRepository(state.db.clone())
            .find_by_id(rule_id)
            .await?
    } else {
        None
    };

    let provider_label = subscription_provider_label(&subscription.provider);
    let provider = providers
        .get_by_name(provider_label)
        .ok_or_else(|| ChannelError::ProviderNotConfigured(subscription.provider.clone()))?;

    let files = resolve_files(state, file_ids).await;
    if !files.is_empty() && !provider.supports_files() {
        return Ok((
            0,
            vec![format!(
                "Provider {:?} does not support file attachments",
                provider.channel_type()
            )],
        ));
    }

    let reply = OutboundReply {
        text: truncate_text(text, provider.max_message_length()),
        task_url: None,
        is_error: false,
        files,
    };

    // Persist a visible stub up front so the row shows in the task timeline
    // and we have a uuid to tag with the outbound `channel_external_id`.
    let stub_uuid =
        persist_outbound_stub_for_subscription(state, task_id, &reply.text, &subscription).await;

    let external_id = match provider
        .send_reply_for_subscription(&subscription, &connection, rule.as_ref(), &reply)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            return Ok((
                0,
                vec![format!(
                    "Failed to send reply via {}: {e}",
                    subscription.provider
                )],
            ));
        }
    };

    if let Some(ref ext_id) = external_id {
        state.recently_sent.note(task_id, ext_id);
        if let Some(ref msg_id) = stub_uuid {
            if let Err(e) = state
                .messages
                .set_channel_metadata(
                    msg_id,
                    ext_id,
                    provider_label,
                    Some(subscription.subject_key.as_str()),
                )
                .await
            {
                warn!(task_id = %task_id, error = %e, "Failed to tag outbound stub with channel_external_id");
            }
        }
        if let Err(e) = subscription_repo
            .touch_cursor(&subscription.id, ext_id)
            .await
        {
            warn!(
                subscription_id = %subscription.id,
                error = %e,
                "Failed to advance subscription cursor"
            );
        }
    }

    Ok((1, vec![]))
}

/// Persist a visible assistant row for a subscription-based outbound reply.
async fn persist_outbound_stub_for_subscription(
    state: &AppState,
    task_id: &str,
    text: &str,
    subscription: &super::models::TaskSubscription,
) -> Option<String> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let uuid = uuid::Uuid::new_v4().to_string();

    let turn_number = match state.messages.current_turn_number(task_id).await {
        Ok(t) => Some(t),
        Err(e) => {
            warn!(task_id, error = %e, "Failed to read turn number for outbound stub");
            None
        }
    };

    let content = serde_json::json!([{ "type": "text", "text": text }]);

    let mut msg =
        TaskMessage::new_user_message(task_id.to_string(), uuid.clone(), content, now.clone());
    msg.message_type = MessageType::System;
    msg.message_role = MessageRole::Assistant;
    msg.user_type = "assistant".to_string();
    msg.turn_number = turn_number;
    msg.channel_provider = Some(subscription_provider_label(&subscription.provider).to_string());
    msg.channel_source_name = Some(subscription.subject_key.clone());

    let block = TaskMessageContentBlock {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: ContentBlockType::Text,
        show_in_ui: true,
        text_content: Some(text.to_string()),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    };

    match state.messages.append(&msg, &[], &[block]).await {
        Ok(_) => Some(uuid),
        Err(e) => {
            warn!(task_id, error = %e, "Failed to persist outbound stub message");
            None
        }
    }
}

fn subscription_provider_label(provider: &str) -> &str {
    match provider {
        "github" => "github_pr",
        other => other,
    }
}
