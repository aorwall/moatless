//! Inbound message processing — unified message → task flow.

use tracing::{info, warn};

use super::config_dao::ChannelConfigPatch;
use super::models::ChannelConfig;
use crate::shared_models::AgentType;
use crate::task::models::{Task, TaskStatus, Visibility};
use crate::state::AppState;

use super::error::ChannelError;
use super::provider::ChannelProviders;
use super::router::{self, RoutingResult};
use super::models::{InboundMessage, MessageStatus, RoutingMode};

/// Process an inbound message through the routing → task creation pipeline.
///
/// Returns the task_id that the message was routed to.
pub async fn process_inbound(
    state: &AppState,
    providers: &ChannelProviders,
    msg: InboundMessage,
) -> Result<String, ChannelError> {
    let routing_result = router::route_message(state, &msg).await?;

    let (task_id, config, thread) = match routing_result {
        RoutingResult::NoConfig => {
            info!(
                provider = msg.provider.as_str(),
                source_id = %msg.source_id,
                "No active channel config found, ignoring message"
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
        } => (task_id, config, thread),
        RoutingResult::NewTask { config } => {
            let task_id = create_task_for_channel(state, &config, &msg).await?;

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

            (task_id, config, None)
        }
    };

    // Create or update thread record
    let thread = if let Some(existing) = thread {
        state.channel_threads.touch(existing.id, Some(&msg.thread_properties)).await?;
        existing
    } else {
        state.channel_threads.insert(
            &config.id,
            &task_id,
            &msg.message_thread_id,
            &msg.thread_properties,
        )
        .await?
    };

    // Set processing status via provider
    if let Some(provider) = providers.get_by_name(&config.provider) {
        if let Err(e) = provider
            .set_status(&thread, &config, MessageStatus::Processing)
            .await
        {
            warn!(error = %e, "Failed to set processing status");
        }
    }

    // Apply prompt template if configured
    let message_text = apply_prompt_template(&config, &msg.text);

    // Send message to the task
    if let Err(e) = crate::message::send::send_message_flow(
        state,
        &task_id,
        &message_text,
        None,
        None,
        &msg.file_ids,
        &[],
    )
    .await
    {
        warn!(task_id = %task_id, error = %e, "Failed to send channel message to task");
        // Try to set error status
        if let Some(provider) = providers.get_by_name(&config.provider) {
            let _ = provider
                .set_status(&thread, &config, MessageStatus::Error)
                .await;
        }
        return Err(ChannelError::Internal(format!(
            "Failed to send message: {e}"
        )));
    }

    // Notify sandbox that a new message is available (fire-and-forget).
    crate::sandbox::notify::notify_sandbox_for_task(state, &task_id).await;

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
        name: None, // Will be auto-named by agent
        description: Some(truncate(&msg.text, 200)),
        site_id: None,
        site_provider: None,
        branch: None, // Will use repo default
        created_by: Some(msg.user_id.clone()),
        visibility: Visibility::Private,
        created_at_db: None,
    };

    state.tasks.insert_task(&row)
        .await
        .map_err(|e| ChannelError::Internal(format!("Failed to create task: {e}")))?;

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

/// Apply prompt template (replace {message} placeholder with actual message).
fn apply_prompt_template(config: &ChannelConfig, message: &str) -> String {
    if let Some(ref template) = config.prompt_template {
        template.replace("{message}", message)
    } else {
        message.to_string()
    }
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
    fn apply_template_with_placeholder() {
        let config = ChannelConfig {
            id: "c1".into(),
            provider: "slack".into(),
            source_id: "C123".into(),
            source_name: None,
            repository_id: "r1".into(),
            routing_mode: RoutingMode::ThreadBased,
            prompt_template: Some("User request:\n\n{message}\n\nPlease help.".into()),
            active: true,
            provider_config: None,
            ongoing_task_id: None,
            created_by: "u1".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let result = apply_prompt_template(&config, "fix the bug");
        assert_eq!(result, "User request:\n\nfix the bug\n\nPlease help.");
    }

    #[test]
    fn apply_template_no_template() {
        let config = ChannelConfig {
            id: "c1".into(),
            provider: "slack".into(),
            source_id: "C123".into(),
            source_name: None,
            repository_id: "r1".into(),
            routing_mode: RoutingMode::ThreadBased,
            prompt_template: None,
            active: true,
            provider_config: None,
            ongoing_task_id: None,
            created_by: "u1".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let result = apply_prompt_template(&config, "fix the bug");
        assert_eq!(result, "fix the bug");
    }

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
