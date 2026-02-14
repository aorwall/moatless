//! Core routing logic: config lookup → task routing based on routing mode.

use super::config_dao as channel_config_dao;
use super::models::{ChannelConfig, ChannelThread};
use super::thread_dao as channel_thread_dao;
use crate::task::dao as task_dao;
use crate::state::AppState;

use super::error::ChannelError;
use super::models::{InboundMessage, RoutingMode};

/// Result of routing an inbound message.
#[derive(Debug)]
pub enum RoutingResult {
    /// No active config found for this provider + source_id.
    NoConfig,
    /// Route to an existing task (thread or ongoing).
    ExistingTask {
        task_id: String,
        thread: Option<ChannelThread>,
        config: ChannelConfig,
    },
    /// Create a new task.
    NewTask {
        config: ChannelConfig,
    },
}

/// Route an inbound message based on config + routing mode.
pub async fn route_message(
    state: &AppState,
    msg: &InboundMessage,
) -> Result<RoutingResult, ChannelError> {
    // 1. Find config
    let config = channel_config_dao::find_by_provider_source(
        &state.db,
        msg.provider.as_str(),
        &msg.source_id,
    )
    .await?;

    let config = match config {
        Some(c) if c.active => c,
        Some(_) => return Ok(RoutingResult::NoConfig), // inactive
        None => return Ok(RoutingResult::NoConfig),
    };

    let routing_mode = RoutingMode::parse(&config.routing_mode)?;

    // 2. Check for existing thread
    let existing_thread = if let Some(ref thread_id) = msg.thread_id {
        channel_thread_dao::find_by_config_and_thread(&state.db, &config.id, thread_id).await?
    } else {
        None
    };

    // 3. Route based on mode + thread existence
    match (routing_mode, existing_thread) {
        // Thread exists → always continue the existing task
        (_, Some(thread)) => Ok(RoutingResult::ExistingTask {
            task_id: thread.task_id.clone(),
            thread: Some(thread),
            config,
        }),

        // Ongoing → find or create
        (RoutingMode::Ongoing, None) => {
            if let Some(ref task_id) = config.ongoing_task_id {
                // Verify task is still open
                if let Ok(Some(task)) =
                    task_dao::find_by_id_internal(&state.db, task_id).await
                {
                    if task.status == crate::task::models::TaskStatus::Open {
                        return Ok(RoutingResult::ExistingTask {
                            task_id: task.task_id.clone(),
                            thread: None,
                            config,
                        });
                    }
                }
            }
            // Ongoing task doesn't exist or is closed → create new
            Ok(RoutingResult::NewTask { config })
        }

        // Per-message or thread-based without existing thread → new task
        (RoutingMode::PerMessage | RoutingMode::ThreadBased, None) => {
            Ok(RoutingResult::NewTask { config })
        }
    }
}
