//! Inbound message processing — unified message → task flow.

use tracing::{info, warn};

use super::config_dao::ChannelConfigPatch;
use super::models::ChannelConfig;
use crate::shared_models::AgentType;
use crate::task::models::{Task, TaskStatus, Visibility};
use crate::state::AppState;

use super::error::ChannelError;
use super::router::{self, RoutingResult};
use super::models::{InboundMessage, RoutingMode};

/// Instructions appended to channel-originated tasks so the agent knows
/// to explicitly reply via the `moat reply` CLI command.
const CHANNEL_REPLY_INSTRUCTIONS: &str = "\
# Channel Reply Instructions

This task was created from a channel message. You MUST send your response back \
to the channel using the `moat reply` command.

- Use `moat reply --text \"<your response>\"` to send your response.
- Always send a reply when done, even if you encountered an error.
- Keep replies concise — the recipient is reading in a messaging app.
- You may send multiple replies for progress updates on long-running tasks.";

/// Process an inbound message through the routing → task creation pipeline.
///
/// Returns the task_id that the message was routed to.
pub async fn process_inbound(
    state: &AppState,
    msg: InboundMessage,
) -> Result<String, ChannelError> {
    let routing_result = router::route_message(state, &msg).await?;

    let mut is_new_task = false;
    let (task_id, config, thread) = match routing_result {
        RoutingResult::NoConfig => {
            warn!(
                provider = msg.provider.as_str(),
                source_id = %msg.source_id,
                user_id = %msg.user_id,
                "Cannot route message: no active channel config found for provider+source"
            );
            return Err(ChannelError::ConfigNotFound(format!(
                "{}:{}",
                msg.provider, msg.source_id
            )));
        }
        RoutingResult::ExistingTask {
            task_id,
            thread,
            config,
        } => {
            info!(
                task_id = %task_id,
                provider = config.provider,
                source_id = %config.source_id,
                routing_mode = ?config.routing_mode,
                has_thread = thread.is_some(),
                "Routing message to existing task"
            );
            (task_id, config, thread)
        }
        RoutingResult::NewTask { config } => {
            info!(
                provider = config.provider,
                source_id = %config.source_id,
                repository_id = %config.repository_id,
                routing_mode = ?config.routing_mode,
                "Creating new task for channel message"
            );
            let task_id = create_task_for_channel(state, &config, &msg).await?;

            // Provision sandbox (mirrors task/handlers.rs)
            match crate::sandbox::auto_provision::build_provision_request(
                state.tasks.as_ref(),
                state.repositories.as_ref(),
                state.provider_tokens.as_ref(),
                state.plugin_repos.as_ref(),
                &task_id,
            )
            .await
            {
                Ok(request) => {
                    match state.sandbox.provision(&task_id, &request).await {
                        Ok(_) => info!(task_id = %task_id, "Sandbox provisioning started for channel task"),
                        Err(e) => warn!(task_id = %task_id, error = %e, "Failed to provision sandbox for channel task"),
                    }
                }
                Err(e) => warn!(task_id = %task_id, error = %e, "Failed to build provision request for channel task"),
            }

            // For ongoing mode, update the config's ongoing_task_id
            if config.routing_mode == RoutingMode::Ongoing {
                state.channel_configs.update(
                    &config.id,
                    &ChannelConfigPatch {
                        ongoing_task_id: Some(Some(task_id.clone())),
                        ..Default::default()
                    },
                )
                .await?;
            }

            is_new_task = true;
            (task_id, config, None)
        }
    };

    // Create or update thread record.
    // For ongoing mode without a thread_id, reuse the existing thread for this
    // task+config to avoid duplicate outbound replies.
    let thread = if let Some(existing) = thread {
        state.channel_threads.touch(existing.id, Some(&msg.thread_properties)).await?;
        existing
    } else if config.routing_mode == RoutingMode::Ongoing {
        // Try to find an existing thread for this task+config before creating a new one
        match state.channel_threads.find_by_task_id_and_config(&task_id, &config.id).await? {
            Some(existing) => {
                state.channel_threads.touch(existing.id, Some(&msg.thread_properties)).await?;
                existing
            }
            None => {
                state.channel_threads.insert(
                    &config.id,
                    &task_id,
                    &msg.message_thread_id,
                    &msg.thread_properties,
                )
                .await?
            }
        }
    } else {
        state.channel_threads.insert(
            &config.id,
            &task_id,
            &msg.message_thread_id,
            &msg.thread_properties,
        )
        .await?
    };

    // Fetch thread context for replies, excluding messages already delivered to the task.
    let thread_context = if msg.thread_id.is_some() {
        if let Some(ref providers) = state.channel_providers {
            if let Some(provider) = providers.get_by_name(&config.provider) {
                let mut exclude_ids = match state.messages.get_channel_external_ids(&task_id).await {
                    Ok(ids) => ids,
                    Err(e) => {
                        warn!(error = %e, "Failed to fetch channel external IDs, fetching full context");
                        vec![]
                    }
                };
                // Also exclude the current message (not yet stored in DB)
                if let Some(ref ext_id) = msg.channel_external_id {
                    exclude_ids.push(ext_id.clone());
                }

                match provider.fetch_context(&thread, &config, 20, &exclude_ids).await {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        warn!(error = %e, "Failed to fetch thread context");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Use the raw message text — prompt_template is passed as append_system_prompt on the task
    let mut message_text = msg.text.clone();
    if let Some(ctx) = thread_context {
        message_text = format!("Thread context (previous messages):\n\n{ctx}\n\n---\n\nNew message:\n\n{message_text}");
    }

    if let Err(e) = crate::message::send::send_message_flow(
        state,
        &task_id,
        &message_text,
        None,
        None,
        &msg.file_ids,
        &[],
        true,
        msg.channel_external_id.as_deref(),
    )
    .await
    {
        warn!(task_id = %task_id, provider = config.provider, source_id = %config.source_id, error = %e, "Failed to send channel message to task");
        return Err(ChannelError::Internal(format!(
            "Failed to send message: {e}"
        )));
    }

    // Notify sandbox that a new message is available (fire-and-forget).
    // Skip for new tasks — the sandbox pod is still starting and will fetch
    // pending messages on startup.
    if !is_new_task {
        crate::sandbox::notify::notify_sandbox_for_task(state, &task_id).await;
    }

    info!(task_id = %task_id, provider = config.provider, "Channel message routed to task");
    Ok(task_id)
}

/// Create a new task from a channel message.
async fn create_task_for_channel(
    state: &AppState,
    config: &ChannelConfig,
    msg: &InboundMessage,
) -> Result<String, ChannelError> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    // Generate short_id
    let short_id = generate_short_id(state).await?;

    let row = Task {
        task_id: task_id.clone(),
        short_id: Some(short_id),
        created_at: now.clone(),
        last_message_at: now,
        agent_type: AgentType(AgentType::CLAUDE_CODE.into()),
        repository_id: config.repository_id.clone(),
        status: TaskStatus::Open,
        total_cost_usd: 0.0,
        site_url: None,
        template_id: None,
        vibe: None,
        name: config.task_name.clone(),
        description: Some(truncate(&msg.text, 200)),
        site_id: None,
        site_provider: None,
        branch: None, // Will use repo default
        created_by: Some(msg.user_id.clone()),
        visibility: Visibility::Private,
        scope: config.scope,
        team_id: config.team_id.clone(),
        created_at_db: None,
        updated_at: None,
        parent_task_id: None,
        append_system_prompt: {
            let mut parts = Vec::new();
            if let Some(ref tpl) = config.prompt_template {
                parts.push(tpl.as_str());
            }
            parts.push(CHANNEL_REPLY_INSTRUCTIONS);
            Some(parts.join("\n\n"))
        },
    };

    state.tasks.insert_task(&row)
        .await
        .map_err(|e| ChannelError::Internal(format!("Failed to create task: {e}")))?;

    // Assign channel tags to the task
    if !config.tag_ids.is_empty() {
        if let Err(e) = state.tags.add_task_tags(&task_id, &config.tag_ids).await {
            warn!(task_id = %task_id, error = %e, "Failed to assign channel tags to task");
        }
    }

    // Emit task event
    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                &task_id,
                moatless_events::TaskSubType::Open,
                Some("claude-code"),
                serde_json::json!({"channel": config.provider, "source_id": config.source_id}),
            )
            .await;
    }

    info!(task_id = %task_id, config_id = %config.id, "Created task from channel message");
    Ok(task_id)
}

/// Manually trigger a channel config, creating a task exactly as if the provider fired.
///
/// Bypasses the active-flag check so inactive channels can still be triggered manually.
/// Always creates a new task (never reuses an ongoing task).
///
/// Returns the `task_id` of the created task.
pub async fn trigger_inbound(
    state: &AppState,
    config: &ChannelConfig,
    message_override: Option<&str>,
) -> Result<String, ChannelError> {
    let message = message_override
        .or_else(|| {
            config
                .provider_config
                .as_ref()
                .and_then(|c| c.get("message"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("Manual channel trigger");

    let user_id = config
        .provider_config
        .as_ref()
        .and_then(|c| c.get("user_id"))
        .and_then(|v| v.as_str())
        .map_or_else(|| config.created_by.clone(), String::from);

    let provider_type = super::provider::ChannelType::parse(&config.provider)
        .unwrap_or(super::provider::ChannelType::Cron);

    let thread_id = uuid::Uuid::new_v4().to_string();

    let msg = InboundMessage {
        provider: provider_type,
        source_id: config.source_id.clone(),
        thread_id: None,
        message_thread_id: thread_id,
        user_id,
        text: message.to_string(),
        file_ids: vec![],
        thread_properties: serde_json::json!({"trigger": "manual"}),
        provider_context: serde_json::json!({}),
        channel_external_id: None,
    };

    info!(
        config_id = %config.id,
        provider = %config.provider,
        "Manually triggering channel"
    );

    let task_id = create_task_for_channel(state, config, &msg).await?;

    // Provision sandbox (mirrors the natural channel fire path)
    match crate::sandbox::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
        &task_id,
    )
    .await
    {
        Ok(request) => {
            match state.sandbox.provision(&task_id, &request).await {
                Ok(_) => info!(task_id = %task_id, "Sandbox provisioning started for triggered channel task"),
                Err(e) => warn!(task_id = %task_id, error = %e, "Failed to provision sandbox for triggered channel task"),
            }
        }
        Err(e) => warn!(task_id = %task_id, error = %e, "Failed to build provision request for triggered channel task"),
    }

    // For ongoing mode, record the new task as the current ongoing task
    if config.routing_mode == RoutingMode::Ongoing {
        state
            .channel_configs
            .update(
                &config.id,
                &ChannelConfigPatch {
                    ongoing_task_id: Some(Some(task_id.clone())),
                    ..Default::default()
                },
            )
            .await?;
    }

    // Create thread record
    state
        .channel_threads
        .insert(
            &config.id,
            &task_id,
            &msg.message_thread_id,
            &msg.thread_properties,
        )
        .await?;

    // Send message to task (sandbox will fetch on startup — is_new_task=true path)
    if let Err(e) = crate::message::send::send_message_flow(
        state,
        &task_id,
        &msg.text,
        None,
        None,
        &msg.file_ids,
        &[],
        true,
        None,
    )
    .await
    {
        warn!(task_id = %task_id, config_id = %config.id, error = %e, "Failed to send trigger message to task");
        return Err(ChannelError::Internal(format!(
            "Failed to send message: {e}"
        )));
    }

    info!(task_id = %task_id, config_id = %config.id, "Channel manually triggered");
    Ok(task_id)
}

/// Generate a unique short ID.
async fn generate_short_id(state: &AppState) -> Result<String, ChannelError> {
    for _ in 0..10 {
        let id = nanoid::nanoid!(8, &nanoid::alphabet::SAFE);
        if !state.tasks.exists_by_short_id(&id)
            .await
            .map_err(|e| ChannelError::Internal(format!("Short ID check failed: {e}")))?
        {
            return Ok(id);
        }
    }
    Err(ChannelError::Internal(
        "Failed to generate unique short ID".to_string(),
    ))
}

/// Truncate a string to a max length, appending "..." if truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long() {
        let long = "a".repeat(300);
        let result = truncate(&long, 200);
        assert!(result.len() <= 200);
        assert!(result.ends_with("..."));
    }
}
