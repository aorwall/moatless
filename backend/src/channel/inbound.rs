//! Inbound message processing — unified message → task flow.

use tracing::{debug, info, warn};

use super::config_dao::ChannelConfigPatch;
use super::models::ChannelConfig;
use crate::shared_models::{AgentMode, AgentType};
use crate::state::AppState;
use crate::task::models::{Task, TaskStatus, Visibility};

use super::error::ChannelError;
use super::models::{InboundMessage, RoutingMode};
use super::router::{self, RoutingResult};

/// Pre-resolved command information passed from provider-specific ingress
/// (e.g., Slack slash commands) or detected from message text.
///
/// When present, [`process_inbound`] uses this to pass command metadata through
/// to `send_message_flow` instead of the default `command_name: None`.
#[derive(Debug, Clone)]
pub struct CommandInfo {
    /// System command name (e.g., "commit", "create-pr").
    pub name: String,
    /// Whether to resume the agent session (`false` when `will_not_resume` is set).
    pub resume: bool,
    /// Agent mode override from the command definition.
    pub agent_mode: Option<AgentMode>,
}

/// Generic instructions appended to all channel-originated tasks so the agent
/// knows to explicitly reply via the `moat reply` CLI command.
///
/// Provider-specific instructions (e.g. Slack mention syntax, file attachments)
/// are appended separately via [`ChannelProvider::reply_instructions`].
const CHANNEL_REPLY_INSTRUCTIONS: &str = "\
# Channel Reply Instructions

This task was created from a channel message. You MUST call `moat reply` to deliver \
your response — a plain text assistant reply is NOT sent to the channel.

## Required: Reply via CLI
For the initial message that created this task, **always** run:
  `moat reply --text \"<your response>\"`

- When this task has exactly one channel, `moat reply --text \"...\"` is sufficient.
- If `moat reply` reports multiple targets, list them with \
`moat channels subscriptions --task $TASK_ID` and pick one with \
`moat reply --thread <id> --text \"...\"` (or `--subscription <id>` / `--pr owner/repo#N`).
- For follow-up thread messages, reply if you are mentioned or have useful information. \
If the message is not directed at you and you have nothing to add, stay silent.
- Keep replies concise — the recipient is reading in a messaging app.
- You may send multiple replies for progress updates on long-running tasks.";

/// Process an inbound message through the routing → task creation pipeline.
///
/// When `command` is `Some`, the command metadata is passed through to
/// `send_message_flow` (name, resume flag, agent_mode). When `None`, the
/// function attempts to detect a `/command` prefix from `msg.text` before
/// sender-label enrichment.
///
/// Returns the task_id that the message was routed to.
pub async fn process_inbound(
    state: &AppState,
    msg: InboundMessage,
    command: Option<CommandInfo>,
) -> Result<String, ChannelError> {
    let routing_result = router::route_message(state, &msg).await?;

    // Resolve command: use provided CommandInfo or detect /command from message text.
    let effective_command = match command {
        Some(cmd) => {
            info!(
                command = %cmd.name,
                provider = msg.provider.as_str(),
                source_id = %msg.source_id,
                "Using provided command for channel message"
            );
            Some(cmd)
        }
        None => detect_command_from_text(&msg.text, state.commands.as_ref()).await,
    };

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
            // Self-echo guard: if this thread has a filter_user_id (or config
            // default) matching msg.user_id, drop it. Otherwise the agent
            // would receive its own outbound `moat reply` back as input.
            if let Some(ref t) = thread {
                if super::listen::should_filter_self_echo(&t.properties, &msg.user_id, &config) {
                    debug!(
                        task_id = %task_id,
                        thread_id = t.id,
                        user_id = %msg.user_id,
                        provider = config.provider,
                        "Filtered self-echo on primary path"
                    );
                    return Ok(task_id);
                }
            } else if !config.effective_user_id().is_empty()
                && config.effective_user_id() == msg.user_id
            {
                debug!(
                    task_id = %task_id,
                    user_id = %msg.user_id,
                    provider = config.provider,
                    "Filtered self-echo on primary path (no thread)"
                );
                return Ok(task_id);
            }

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
            let task_id =
                create_task_for_channel(state, &config, &msg, effective_command.as_ref()).await?;

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
                Ok(request) => match state.sandbox.provision(&task_id, &request).await {
                    Ok(_) => {
                        info!(task_id = %task_id, "Sandbox provisioning started for channel task");
                    }
                    Err(e) => {
                        warn!(task_id = %task_id, error = %e, "Failed to provision sandbox for channel task");
                    }
                },
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Failed to build provision request for channel task");
                }
            }

            // For ongoing mode, update the config's ongoing_task_id
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

            is_new_task = true;
            (task_id, config, None)
        }
    };

    // Create or update thread record.
    // For ongoing mode without a thread_id, reuse the existing thread for this
    // task+config to avoid duplicate outbound replies.
    //
    // When the config has response_config_id (e.g. cron → Slack), create the
    // thread under the response config so outbound replies go directly there.
    // The thread is marked pending until the first outbound message is sent.
    let thread_config_id = config.response_config_id.as_deref().unwrap_or(&config.id);
    let thread_properties = if config.response_config_id.is_some() {
        let mut props = msg.thread_properties.clone();
        if let Some(obj) = props.as_object_mut() {
            obj.insert("pending".into(), serde_json::json!(true));
        }
        props
    } else {
        msg.thread_properties.clone()
    };

    let thread = if let Some(existing) = thread {
        state
            .channel_threads
            .touch(existing.id, Some(&msg.thread_properties))
            .await?;
        existing
    } else if config.routing_mode == RoutingMode::Ongoing {
        // Try to find an existing thread for this task+config before creating a new one
        match state
            .channel_threads
            .find_by_task_id_and_config(&task_id, thread_config_id)
            .await?
        {
            Some(existing) => {
                state
                    .channel_threads
                    .touch(existing.id, Some(&msg.thread_properties))
                    .await?;
                existing
            }
            None => {
                state
                    .channel_threads
                    .insert(
                        thread_config_id,
                        &task_id,
                        &msg.message_thread_id,
                        &thread_properties,
                    )
                    .await?
            }
        }
    } else {
        state
            .channel_threads
            .insert(
                thread_config_id,
                &task_id,
                &msg.message_thread_id,
                &thread_properties,
            )
            .await?
    };

    deliver_to_task(
        state,
        &task_id,
        &config,
        &thread,
        &msg,
        effective_command.as_ref(),
        is_new_task,
    )
    .await?;

    info!(task_id = %task_id, provider = config.provider, "Channel message routed to task");
    Ok(task_id)
}

pub(crate) async fn deliver_to_task(
    state: &AppState,
    task_id: &str,
    config: &ChannelConfig,
    thread: &crate::channel::models::ChannelThread,
    msg: &InboundMessage,
    command: Option<&CommandInfo>,
    is_new_task: bool,
) -> Result<(), ChannelError> {
    // Fetch thread context for replies, excluding messages already delivered to the task.
    let thread_context = if msg.thread_id.is_some() {
        if let Some(ref providers) = state.channel_providers {
            if let Some(provider) = providers.get_by_name(&config.provider) {
                let mut exclude_ids = match state.messages.get_channel_external_ids(task_id).await {
                    Ok(ids) => ids,
                    Err(e) => {
                        warn!(error = %e, "Failed to fetch channel external IDs, fetching full context");
                        vec![]
                    }
                };
                if let Some(ref ext_id) = msg.channel_external_id {
                    exclude_ids.push(ext_id.clone());
                }

                match provider
                    .fetch_context(thread, config, 20, &exclude_ids)
                    .await
                {
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

    let has_cursor = thread
        .properties
        .get("last_channel_external_id")
        .and_then(|v| v.as_str())
        .is_some();

    let base_text = match command {
        Some(cmd) => {
            let prefix = format!("/{}", cmd.name);
            msg.text
                .trim()
                .strip_prefix(&prefix)
                .unwrap_or(&msg.text)
                .trim()
                .to_string()
        }
        None => msg.text.clone(),
    };

    let mut message_text = base_text;
    let sender_label = match state.users.get_user(&msg.user_id).await {
        Ok(Some(user)) => {
            let display = user.name.as_deref().unwrap_or(&user.login);
            let provider_user_id = msg
                .provider_context
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if provider_user_id.is_empty() {
                Some(format!("{display} ({login})", login = user.login))
            } else {
                Some(format!(
                    "{display} ({login}, Slack: {provider_user_id})",
                    login = user.login
                ))
            }
        }
        _ => None,
    };
    if let Some(label) = sender_label {
        message_text = format!("{label}: {message_text}");
    }

    let is_bot_mentioned = msg
        .provider_context
        .get("is_bot_mentioned")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if is_bot_mentioned {
        message_text = format!(
            "[You were @mentioned — a reply via `moat reply` is required]\n\n{message_text}"
        );
    }

    if msg.thread_id.is_some() {
        if let Some(providers) = state.channel_providers.as_ref() {
            if let Some(provider) = providers.get_by_name(&config.provider) {
                if let Some(hint) = provider.thread_reply_hint(msg) {
                    message_text = format!("{hint}\n\n{message_text}");
                }
            }
        }
    }

    if let Some(ctx) = thread_context {
        if has_cursor {
            message_text = format!("{ctx}\n\n---\n\n{message_text}");
        } else {
            message_text = format!(
                "Thread context (previous messages):\n\n{ctx}\n\n---\n\nNew message:\n\n{message_text}"
            );
        }
    }

    let (cmd_resume, cmd_agent_mode, cmd_name) = match command {
        Some(cmd) => (cmd.resume, cmd.agent_mode.as_ref(), Some(cmd.name.as_str())),
        None => (true, None, None),
    };

    if let Err(e) = crate::message::send::send_message_flow(
        state,
        task_id,
        &message_text,
        None,
        cmd_agent_mode,
        &msg.file_ids,
        &[],
        cmd_resume,
        msg.channel_external_id.as_deref(),
        Some(&config.provider),
        config.source_name.as_deref(),
        cmd_name,
        None,
        false,
    )
    .await
    {
        warn!(task_id, provider = config.provider, source_id = %config.source_id, error = %e, "Failed to send channel message to task");
        return Err(ChannelError::Internal(format!(
            "Failed to send message: {e}"
        )));
    }

    if let Some(ref ext_id) = msg.channel_external_id {
        if let Err(e) = state
            .channel_threads
            .touch(
                thread.id,
                Some(&serde_json::json!({"last_channel_external_id": ext_id})),
            )
            .await
        {
            warn!(task_id, error = %e, "Failed to update last_channel_external_id on thread");
        }
    }

    if !is_new_task {
        if let Err(e) = crate::sandbox::notify::ensure_sandbox_and_notify(state, task_id).await {
            warn!(task_id, error = %e, "Failed to ensure sandbox for channel message");
        }
    }

    Ok(())
}

/// Create a new task from a channel message.
pub(crate) async fn create_task_for_channel(
    state: &AppState,
    config: &ChannelConfig,
    msg: &InboundMessage,
    command: Option<&CommandInfo>,
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
        agent_type: AgentType(config.agent_type.clone()),
        repository_id: config.repository_id.clone(),
        status: TaskStatus::Open,
        total_cost_usd: 0.0,
        site_url: None,
        template_id: None,
        vibe: None,
        name: config.task_name.clone(),
        description: Some(match command {
            Some(cmd) => {
                let stripped = msg
                    .text
                    .trim()
                    .strip_prefix(&format!("/{}", cmd.name))
                    .unwrap_or(&msg.text)
                    .trim();
                if stripped.is_empty() {
                    truncate(&format!("/{}", cmd.name), 200)
                } else {
                    truncate(stripped, 200)
                }
            }
            None => truncate(&msg.text, 200),
        }),
        site_id: None,
        site_provider: None,
        branch: None, // Will use repo default
        created_by: Some(config.effective_user_id().to_string()),
        visibility: Visibility::Private,
        scope: config.scope,
        team_id: config.team_id.clone(),
        created_at_db: None,
        updated_at: None,
        parent_task_id: None,
        append_system_prompt: {
            let mut parts = Vec::new();
            if let Some(ref tpl) = config.prompt_template {
                parts.push(tpl.to_string());
            }
            parts.push(CHANNEL_REPLY_INSTRUCTIONS.to_string());
            // Append provider-specific reply instructions (e.g. Slack mention syntax)
            if let Some(providers) = state.channel_providers.as_ref() {
                if let Some(provider) = providers.get_by_name(&config.provider) {
                    if let Some(extra) = provider.reply_instructions() {
                        parts.push(extra);
                    }
                }
            }
            Some(parts.join("\n\n"))
        },
        sandbox_started_at: None,
        headless: true,
        ttl_minutes: crate::task::models::DEFAULT_TTL_MINUTES,
    };

    state
        .tasks
        .insert_task(&row)
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
    thread_properties: Option<serde_json::Value>,
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
        .map_or_else(|| config.effective_user_id().to_string(), String::from);

    let provider_type = super::provider::ChannelType::parse(&config.provider)
        .unwrap_or(super::provider::ChannelType::Cron);

    if provider_type == super::provider::ChannelType::GithubPr {
        return Err(ChannelError::BadRequest(
            "GitHub channels are not triggerable via channel_configs; \
             use the rules-based ingress flow instead"
                .into(),
        ));
    }

    let resolved_thread_properties =
        thread_properties.unwrap_or_else(|| serde_json::json!({"trigger": "manual"}));

    let thread_id = uuid::Uuid::new_v4().to_string();

    let msg = InboundMessage {
        provider: provider_type,
        source_id: config.source_id.clone(),
        thread_id: None,
        message_thread_id: thread_id,
        user_id,
        text: message.to_string(),
        file_ids: vec![],
        thread_properties: resolved_thread_properties,
        provider_context: serde_json::json!({}),
        channel_external_id: None,
    };

    info!(
        config_id = %config.id,
        provider = %config.provider,
        "Manually triggering channel"
    );

    let task_id = create_task_for_channel(state, config, &msg, None).await?;

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
        Ok(request) => match state.sandbox.provision(&task_id, &request).await {
            Ok(_) => {
                info!(task_id = %task_id, "Sandbox provisioning started for triggered channel task");
            }
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to provision sandbox for triggered channel task");
            }
        },
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to build provision request for triggered channel task");
        }
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

    // Create thread record — use response config if set (e.g. cron → Slack)
    let thread_config_id = config.response_config_id.as_deref().unwrap_or(&config.id);
    let thread_properties = if config.response_config_id.is_some() {
        let mut props = msg.thread_properties.clone();
        if let Some(obj) = props.as_object_mut() {
            obj.insert("pending".into(), serde_json::json!(true));
        }
        props
    } else {
        msg.thread_properties.clone()
    };
    state
        .channel_threads
        .insert(
            thread_config_id,
            &task_id,
            &msg.message_thread_id,
            &thread_properties,
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
        Some(&config.provider),
        config.source_name.as_deref(),
        None,
        None,
        false,
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
        if !state
            .tasks
            .exists_by_short_id(&id)
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

/// Detect a `/command` prefix from raw message text and look it up in the DB.
///
/// Returns `Some(CommandInfo)` if the text starts with `/word` and that word
/// matches a known, active system command.
async fn detect_command_from_text(
    text: &str,
    commands: &dyn crate::command::dao::CommandRepository,
) -> Option<CommandInfo> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let without_slash = &trimmed[1..];
    let candidate = without_slash.split_whitespace().next()?;

    match commands.find_by_name(candidate).await {
        Ok(Some(cmd)) if cmd.active != Some(false) => {
            debug!(command_name = %candidate, "Detected command from channel message text");
            Some(CommandInfo {
                name: cmd.name,
                resume: !cmd.will_not_resume.unwrap_or(false),
                agent_mode: cmd.agent_mode,
            })
        }
        _ => None,
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
