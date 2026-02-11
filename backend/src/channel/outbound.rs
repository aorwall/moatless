//! Outbound channel notifications — event-driven replies to channel threads.

use tracing::{debug, warn};

use crate::db::{channel_config_dao, channel_thread_dao};
use crate::event_handler::types::AgentEvent;
use crate::state::AppState;

use super::provider::ChannelProviders;
use super::types::{MessageStatus, OutboundReply};

/// Handle a channel notification triggered by an agent event.
///
/// Called from `event_handler/consumer.rs` after standard event processing.
/// Looks up channel threads for the task and sends replies/status updates.
pub async fn handle_channel_notification(
    state: &AppState,
    providers: &ChannelProviders,
    event: &AgentEvent,
) {
    // Find threads for this task
    let threads = match channel_thread_dao::find_by_task_id(&state.db, &event.task_id).await {
        Ok(t) => t,
        Err(e) => {
            debug!(task_id = %event.task_id, error = %e, "Failed to look up channel threads");
            return;
        }
    };

    if threads.is_empty() {
        return; // Task not from a channel
    }

    let key = format!("{}.{}", event.event_type, event.sub_type);

    for thread in &threads {
        // Load config for this thread
        let config = match channel_config_dao::find_by_id(&state.db, &thread.config_id).await {
            Ok(Some(c)) => c,
            _ => continue,
        };

        let provider = match providers.get_by_name(&config.provider) {
            Some(p) => p,
            None => continue,
        };

        let task_url = build_task_url(state, &event.task_id);

        match key.as_str() {
            "taskTurn.completed" | "agent.completed" => {
                // Extract result text
                let result_text = event
                    .payload
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Task completed");

                let reply = OutboundReply {
                    text: truncate_for_slack(result_text, 2000),
                    task_url: Some(task_url),
                    is_error: false,
                };

                if let Err(e) = provider.send_reply(thread, &config, &reply).await {
                    warn!(task_id = %event.task_id, error = %e, "Failed to send channel reply");
                }
                if let Err(e) = provider
                    .set_status(thread, &config, MessageStatus::Completed)
                    .await
                {
                    warn!(task_id = %event.task_id, error = %e, "Failed to set completed status");
                }
            }

            "agent.error" | "taskTurn.error" => {
                let error_msg = event
                    .payload
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("An error occurred");

                let reply = OutboundReply {
                    text: format!("Error: {}", truncate_for_slack(error_msg, 1900)),
                    task_url: Some(task_url),
                    is_error: true,
                };

                if let Err(e) = provider.send_reply(thread, &config, &reply).await {
                    warn!(task_id = %event.task_id, error = %e, "Failed to send error reply");
                }
                if let Err(e) = provider
                    .set_status(thread, &config, MessageStatus::Error)
                    .await
                {
                    warn!(task_id = %event.task_id, error = %e, "Failed to set error status");
                }
            }

            _ => {
                debug!(
                    task_id = %event.task_id,
                    event_key = %key,
                    "Event type not handled for channel notifications"
                );
            }
        }
    }
}

/// Build the task URL for linking in channel replies.
fn build_task_url(state: &AppState, task_id: &str) -> String {
    format!("{}/tasks/{}", state.config.frontend_url, task_id)
}

/// Truncate text for Slack messages (max ~4000 chars, but we use a safe limit).
fn truncate_for_slack(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        format!("{}...", &text[..max.saturating_sub(3)])
    }
}
