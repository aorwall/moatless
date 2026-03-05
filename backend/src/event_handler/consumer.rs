//! Redis stream consumer for agent events.
//!
//! Subscribes to `task-events:*` streams using a separate consumer group
//! from the SSE consumer. Processes `agent.*` and `task.metadata.updated` events.

use redis::aio::ConnectionManager;
use redis::Value;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use moatless_events::{AgentSubType, SandboxSubType, TaskSubType, TaskTurnSubType};

use crate::state::AppState;

use super::agent_handlers;
use super::models::{AgentEvent, EventKey};

/// Consumer group name — separate from the SSE consumer group.
const CONSUMER_GROUP: &str = "event-handler";

/// Redis stream consumer for processing agent lifecycle events.
pub struct EventHandlerConsumer {
    conn: ConnectionManager,
    consumer_id: String,
    state: AppState,
    cancel: CancellationToken,
}

impl EventHandlerConsumer {
    pub fn new(
        conn: ConnectionManager,
        consumer_id: String,
        state: AppState,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            conn,
            consumer_id,
            state,
            cancel,
        }
    }

    /// Run the consumer loop until cancelled.
    pub async fn run(mut self) {
        info!(
            consumer_id = %self.consumer_id,
            group = CONSUMER_GROUP,
            "Event handler consumer started"
        );

        loop {
            if self.cancel.is_cancelled() {
                info!("Event handler consumer shutting down");
                break;
            }

            if let Err(e) = self.consume_cycle().await {
                if self.cancel.is_cancelled() {
                    break;
                }
                warn!(error = %e, "Redis connection lost in event handler consumer, retrying in 2s");
                tokio::select! {
                    () = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                    () = self.cancel.cancelled() => { break; }
                }
            }
        }

        info!("Event handler consumer stopped");
    }

    /// One consume cycle: discover streams, ensure groups, read + process + ACK.
    async fn consume_cycle(&mut self) -> Result<(), redis::RedisError> {
        let streams = self.scan_streams().await?;

        if streams.is_empty() {
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                () = self.cancel.cancelled() => {}
            }
            return Ok(());
        }

        for stream in &streams {
            self.ensure_group(stream).await;
        }

        // XREADGROUP
        let mut cmd = redis::cmd("XREADGROUP");
        cmd.arg("GROUP")
            .arg(CONSUMER_GROUP)
            .arg(&self.consumer_id)
            .arg("BLOCK")
            .arg(2000_u64)
            .arg("COUNT")
            .arg(50_u64)
            .arg("STREAMS");

        for stream in &streams {
            cmd.arg(stream);
        }
        for _ in &streams {
            cmd.arg(">");
        }

        let result: Value = cmd.query_async(&mut self.conn).await?;
        self.process_response(&result).await;

        Ok(())
    }

    /// SCAN for keys matching `task-events:*`.
    async fn scan_streams(&mut self) -> Result<Vec<String>, redis::RedisError> {
        let mut streams = Vec::new();
        let mut cursor = 0_u64;

        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg("task-events:*")
                .arg("COUNT")
                .arg(100)
                .query_async(&mut self.conn)
                .await?;

            streams.extend(keys);
            cursor = next_cursor;

            if cursor == 0 {
                break;
            }
        }

        Ok(streams)
    }

    /// Ensure the consumer group exists on a stream.
    async fn ensure_group(&mut self, stream: &str) {
        let result: Result<String, redis::RedisError> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream)
            .arg(CONSUMER_GROUP)
            .arg("$")
            .arg("MKSTREAM")
            .query_async(&mut self.conn)
            .await;

        if let Err(e) = result {
            let msg = e.to_string();
            if !msg.contains("BUSYGROUP") {
                warn!(stream = %stream, error = %e, "Failed to create consumer group");
            }
        }
    }

    /// Parse XREADGROUP response and dispatch events to handlers.
    async fn process_response(&mut self, value: &Value) {
        let stream_results = match value {
            Value::Array(arr) => arr,
            Value::Nil => return,
            _ => {
                warn!("Unexpected XREADGROUP response format");
                return;
            }
        };

        for stream_result in stream_results {
            let (stream_name, entries) = match stream_result {
                Value::Array(pair) if pair.len() == 2 => {
                    let name = extract_string(&pair[0]).unwrap_or_default();
                    let entries = match &pair[1] {
                        Value::Array(e) => e,
                        _ => continue,
                    };
                    (name, entries)
                }
                _ => continue,
            };

            let mut entry_ids = Vec::new();

            for entry in entries {
                let (entry_id, fields) = match entry {
                    Value::Array(pair) if pair.len() == 2 => {
                        let id = extract_string(&pair[0]).unwrap_or_default();
                        let fields = match &pair[1] {
                            Value::Array(f) => f,
                            _ => continue,
                        };
                        (id, fields)
                    }
                    _ => continue,
                };

                // Extract the `payload` field and parse as AgentEvent
                if let Some(payload_str) = extract_field(fields, "payload") {
                    if let Ok(event) = serde_json::from_str::<AgentEvent>(&payload_str) {
                        self.dispatch_event(&event).await;
                    } else {
                        warn!(entry_id = %entry_id, "Failed to parse event payload, ACKing to prevent redelivery");
                    }
                }

                entry_ids.push(entry_id);
            }

            // ACK all processed entries
            if !entry_ids.is_empty() {
                let ack_result: Result<i64, redis::RedisError> = {
                    let mut cmd = redis::cmd("XACK");
                    cmd.arg(&stream_name).arg(CONSUMER_GROUP);
                    for id in &entry_ids {
                        cmd.arg(id);
                    }
                    cmd.query_async(&mut self.conn).await
                };

                if let Err(e) = ack_result {
                    warn!(
                        stream = %stream_name,
                        count = entry_ids.len(),
                        error = %e,
                        "Failed to ACK entries"
                    );
                }
            }
        }
    }

    /// Dispatch a parsed event to the appropriate handler.
    async fn dispatch_event(&self, event: &AgentEvent) {
        match event.event_key() {
            EventKey::TaskTurn(TaskTurnSubType::Completed)
            | EventKey::Agent(AgentSubType::Completed) => {
                agent_handlers::handle_agent_completed(&self.state, event).await;
                self.notify_channels(event).await;
            }
            EventKey::Agent(AgentSubType::Message) => {
                agent_handlers::handle_agent_message(&self.state, event).await;
            }
            EventKey::Agent(AgentSubType::SessionInit) => {
                agent_handlers::handle_agent_session_init(&self.state, event).await;
            }
            EventKey::Agent(AgentSubType::Stopped)
            | EventKey::TaskTurn(TaskTurnSubType::Stopped) => {
                agent_handlers::handle_agent_stopped(&self.state, event).await;
            }
            EventKey::Agent(AgentSubType::Error)
            | EventKey::TaskTurn(TaskTurnSubType::Error) => {
                agent_handlers::handle_agent_error(&self.state, event).await;
                self.notify_channels(event).await;
            }
            EventKey::Task(TaskSubType::MetadataUpdated) => {
                agent_handlers::handle_task_metadata_updated(&self.state, event).await;
            }
            // Sandbox error — pod watcher detected a fatal crash (OOM, crash loop, etc.)
            EventKey::Sandbox(SandboxSubType::Error) => {
                agent_handlers::handle_sandbox_fatal(&self.state, event).await;
            }
            // Known SSE-only event types — forwarded to frontend but not processed here
            EventKey::Sandbox(_)
            | EventKey::Server(_)
            | EventKey::Agent(AgentSubType::Started | AgentSubType::PendingResponse)
            | EventKey::Task(
                TaskSubType::Open
                | TaskSubType::Created
                | TaskSubType::Closed
                | TaskSubType::Updated,
            )
            | EventKey::TaskTurn(TaskTurnSubType::Active) => {
                debug!(
                    task_id = %event.task_id,
                    event_type = %event.event_type,
                    sub_type = %event.sub_type,
                    "SSE-only event type, skipping"
                );
            }
            EventKey::Unknown(ref t, ref s) => {
                warn!(
                    task_id = %event.task_id,
                    event_type = %t,
                    sub_type = %s,
                    "Unhandled event type — event not processed"
                );
            }
        }
    }

    /// Notify channel providers about task events (if any channels are configured).
    async fn notify_channels(&self, event: &AgentEvent) {
        if let Some(ref providers) = self.state.channel_providers {
            crate::channel::outbound::handle_channel_notification(
                &self.state,
                providers,
                event,
            )
            .await;
        }
    }
}

/// Extract a String from a Redis Value.
fn extract_string(v: &Value) -> Option<String> {
    match v {
        Value::BulkString(bytes) => String::from_utf8(bytes.clone()).ok(),
        Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

/// Extract a named field value from a flat [key, val, key, val, ...] array.
fn extract_field(fields: &[Value], name: &str) -> Option<String> {
    let mut iter = fields.iter();
    while let Some(key) = iter.next() {
        let val = iter.next()?;
        if let Some(key_str) = extract_string(key) {
            if key_str == name {
                return extract_string(val);
            }
        }
    }
    None
}
