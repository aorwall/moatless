//! Task event handler — background worker that processes agent lifecycle events.
//!
//! Subscribes to Redis streams and handles agent.completed, agent.stopped,
//! agent.error, agent.message (auto-tagging), and task.metadata.updated events.

pub mod agent_handlers;
pub mod consumer;
pub mod metrics;
pub mod models;
pub mod skeleton;

use std::sync::Arc;

use redis::aio::ConnectionManager;
use tokio_util::sync::CancellationToken;
use tracing::info;

use moatless_loops::adapters::events::TaskEventSubscribers;

use crate::state::AppState;

/// Spawn the event handler consumer as a background task.
///
/// Returns a future that runs the consumer loop until cancellation.
pub async fn spawn_event_handler(
    conn: ConnectionManager,
    state: AppState,
    subscribers: Arc<TaskEventSubscribers>,
    cancel: CancellationToken,
) {
    let consumer_id = format!(
        "event-handler-{}",
        std::env::var("HOSTNAME").unwrap_or_else(|_| "local".to_string())
    );

    info!(consumer_id = %consumer_id, "Spawning event handler consumer");

    let consumer =
        consumer::EventHandlerConsumer::new(conn, consumer_id, state, subscribers, cancel);
    consumer.run().await;
}
