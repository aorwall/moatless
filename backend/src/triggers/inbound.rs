//! Inbound message processing — unified message → task flow.

use tracing::{info, warn};

use crate::adapters::models::TriggerContext;
use crate::shared_models::{AgentMode, AgentType};
use crate::state::AppState;
use crate::task::models::{Task, TaskStatus, Visibility};

use crate::adapters::error::AdapterError;
use crate::adapters::models::InboundMessage;

/// Pre-resolved command information passed from provider-specific event handling
/// (e.g., Slack slash commands) or detected from message text.
///
/// When present, provider event delivery uses this to pass command metadata through
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

/// Generic instructions appended to all external-source tasks so the agent
/// knows to explicitly reply via the `moat reply` CLI command.
///
/// Provider-specific instructions (e.g. Slack mention syntax, file attachments)
/// are appended separately via [`Adapter::reply_instructions`].
const EXTERNAL_REPLY_INSTRUCTIONS: &str = "\
# External Reply Instructions

This task was created from an external message. You MUST call `moat reply` to deliver \
your response — a plain text assistant reply is NOT sent to the external target.

## Required: Reply via CLI
For the initial message that created this task, **always** run:
  `moat reply --text \"<your response>\"`

- When this task has exactly one reply target, `moat reply --text \"...\"` is sufficient.
- If `moat reply` reports multiple targets, list them with \
`moat bindings --task $TASK_ID` and pick one with \
`moat reply --binding <id> --text \"...\"`.
- For follow-up thread messages, reply if you are mentioned or have useful information. \
If the message is not directed at you and you have nothing to add, stay silent.
- Keep replies concise — the recipient is reading in a messaging app.
- You may send multiple replies for progress updates on long-running tasks.";

/// Create a new task from a external message.
pub(crate) async fn create_task_for_external_message(
    state: &AppState,
    config: &TriggerContext,
    msg: &InboundMessage,
    command: Option<&CommandInfo>,
) -> Result<String, AdapterError> {
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
        owner_user_id: (config.scope == crate::scope::Scope::User)
            .then(|| config.effective_user_id().to_string()),
        created_at_db: None,
        updated_at: None,
        parent_task_id: None,
        append_system_prompt: {
            let mut parts = Vec::new();
            if let Some(ref tpl) = config.prompt_template {
                parts.push(tpl.to_string());
            }
            parts.push(EXTERNAL_REPLY_INSTRUCTIONS.to_string());
            // Append provider-specific reply instructions (e.g. Slack mention syntax)
            if let Some(providers) = state.adapters.as_ref() {
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
        inactive_ttl_days: crate::task::models::DEFAULT_INACTIVE_TTL_DAYS,
        task_template_id: None,
    };

    state
        .tasks
        .insert_task(&row)
        .await
        .map_err(|e| AdapterError::Internal(format!("Failed to create task: {e}")))?;

    // Assign trigger tags to the task
    if !config.tag_ids.is_empty() {
        if let Err(e) = state.tags.add_task_tags(&task_id, &config.tag_ids).await {
            warn!(task_id = %task_id, error = %e, "Failed to assign trigger tags to task");
        }
    }

    // Emit task event
    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                &task_id,
                moatless_events::TaskSubType::Open,
                Some("claude-code"),
                serde_json::json!({"adapter": config.provider, "source_id": config.source_id}),
            )
            .await;
    }

    info!(task_id = %task_id, trigger_id = %config.id, "Created task from external message");
    Ok(task_id)
}

/// Generate a unique short ID.
async fn generate_short_id(state: &AppState) -> Result<String, AdapterError> {
    for _ in 0..10 {
        let id = nanoid::nanoid!(8, &nanoid::alphabet::SAFE);
        if !state
            .tasks
            .exists_by_short_id(&id)
            .await
            .map_err(|e| AdapterError::Internal(format!("Short ID check failed: {e}")))?
        {
            return Ok(id);
        }
    }
    Err(AdapterError::Internal(
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
