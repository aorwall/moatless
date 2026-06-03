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
use crate::task_event::dao::InsertTaskEvent;

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
    /// When set, the next cycle first drains this consumer's Pending Entries
    /// List (delivered-but-not-ACKed entries) before reading new entries.
    /// Set on startup and after every connection error so an `agent.completed`
    /// lost mid-cycle during Redis flapping is recovered rather than orphaned.
    recover_pending: bool,
}

/// Which slice of a stream a `XREADGROUP` should read.
#[derive(Clone, Copy)]
enum ReadMode {
    /// New, never-delivered entries (`>`), blocking briefly for arrivals.
    New,
    /// This consumer's pending (delivered-but-unacked) entries (`0`).
    Pending,
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
            recover_pending: true,
        }
    }

    /// Run the consumer loop until cancelled.
    pub async fn run(mut self) {
        info!(
            consumer_id = %self.consumer_id,
            group = CONSUMER_GROUP,
            "Event handler consumer started"
        );

        let mut failures: u32 = 0;
        loop {
            if self.cancel.is_cancelled() {
                info!("Event handler consumer shutting down");
                break;
            }

            match self.consume_cycle().await {
                Ok(()) => failures = 0,
                Err(e) => {
                    if self.cancel.is_cancelled() {
                        break;
                    }
                    failures = failures.saturating_add(1);
                    // Recover the PEL on the next successful cycle — entries
                    // delivered but not ACKed before this drop must be reclaimed.
                    self.recover_pending = true;
                    let delay = crate::events::reconnect_backoff_delay(failures);
                    warn!(
                        error = %e,
                        failures,
                        delay_ms = delay.as_millis() as u64,
                        "Redis connection lost in event handler consumer, backing off"
                    );
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {}
                        () = self.cancel.cancelled() => { break; }
                    }
                }
            }
        }

        info!("Event handler consumer stopped");
    }

    /// One consume cycle: discover streams, ensure groups, recover pending,
    /// then read + process + ACK new entries.
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

        // PEL recovery: drain entries previously delivered to this consumer but
        // never ACKed (e.g. a connection drop mid-cycle) before reading new ones.
        if self.recover_pending {
            self.recover_pending_entries(&streams).await?;
            self.recover_pending = false;
        }

        let result = self.read_group(&streams, ReadMode::New).await?;
        self.process_response(&result).await;

        Ok(())
    }

    /// Drain this consumer's Pending Entries List across all streams, processing
    /// and ACKing until empty. Reading with id `0` returns the PEL from the
    /// start; since each pass ACKs what it processes, successive reads return
    /// fewer entries until drained. Capped to avoid an unbounded loop on a
    /// poison entry (those are ACKed by `process_response` regardless).
    async fn recover_pending_entries(
        &mut self,
        streams: &[String],
    ) -> Result<(), redis::RedisError> {
        let mut total = 0usize;
        for _ in 0..100 {
            let result = self.read_group(streams, ReadMode::Pending).await?;
            let count = self.process_response(&result).await;
            total += count;
            if count == 0 {
                break;
            }
        }
        if total > 0 {
            info!(
                group = CONSUMER_GROUP,
                recovered = total,
                "Recovered pending Redis entries after reconnect"
            );
        }
        Ok(())
    }

    /// Issue one `XREADGROUP` for the given streams in the given mode.
    async fn read_group(
        &mut self,
        streams: &[String],
        mode: ReadMode,
    ) -> Result<Value, redis::RedisError> {
        let mut cmd = redis::cmd("XREADGROUP");
        cmd.arg("GROUP").arg(CONSUMER_GROUP).arg(&self.consumer_id);
        if matches!(mode, ReadMode::New) {
            cmd.arg("BLOCK").arg(2000_u64);
        }
        cmd.arg("COUNT").arg(50_u64).arg("STREAMS");

        for stream in streams {
            cmd.arg(stream);
        }
        let id = match mode {
            ReadMode::New => ">",
            ReadMode::Pending => "0",
        };
        for _ in streams {
            cmd.arg(id);
        }

        cmd.query_async(&mut self.conn).await
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

    /// Parse XREADGROUP response and dispatch events to handlers. Returns the
    /// number of entries ACKed, letting the PEL drain loop detect an empty read.
    async fn process_response(&mut self, value: &Value) -> usize {
        let stream_results = match value {
            Value::Array(arr) => arr,
            Value::Nil => return 0,
            _ => {
                warn!("Unexpected XREADGROUP response format");
                return 0;
            }
        };

        let mut processed = 0usize;

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
                        self.persist_event(&event).await;
                        self.dispatch_event(&event).await;
                    } else {
                        warn!(entry_id = %entry_id, "Failed to parse event payload, ACKing to prevent redelivery");
                    }
                }

                entry_ids.push(entry_id);
            }

            // ACK all processed entries
            if !entry_ids.is_empty() {
                processed += entry_ids.len();
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

        processed
    }

    /// Persist an event to the database (best-effort, never blocks dispatch).
    async fn persist_event(&self, event: &AgentEvent) {
        let insert = InsertTaskEvent {
            event_id: event.event_id.clone(),
            task_id: event.task_id.clone(),
            event_type: event.event_type.as_str().to_string(),
            sub_type: event.sub_type.clone(),
            is_error: event.is_error,
            agent_type: event.agent_type.clone(),
            turn_number: event.turn_number,
            payload: event.payload.clone(),
            timestamp: event.timestamp.clone(),
        };
        if let Err(e) = self.state.task_events.insert_event(&insert).await {
            warn!(
                task_id = %event.task_id,
                event_id = %event.event_id,
                error = %e,
                "Failed to persist event to database"
            );
        }
    }

    /// Dispatch a parsed event to the appropriate handler.
    async fn dispatch_event(&self, event: &AgentEvent) {
        match event.event_key() {
            EventKey::Agent(AgentSubType::Completed) => {
                // handle_agent_completed publishes task_turn.completed; external
                // notification is handled by the agent via `moat reply`.
                agent_handlers::handle_agent_completed(&self.state, event).await;
            }
            EventKey::TaskTurn(TaskTurnSubType::Completed) => {
                // External notification is handled by the agent via `moat reply`.
            }
            EventKey::Agent(AgentSubType::Message) => {
                agent_handlers::handle_agent_message(&self.state, event).await;
                // Forward content blocks to Linear:
                // - assistant role → persistent `thought` text + ephemeral
                //   pending `action` per tool_use.
                // - user role → non-ephemeral completed `action` per
                //   tool_result, looked up against the persisted tool_use row
                //   to recover the original tool name and input summary.
                let role = event
                    .payload
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("assistant");
                if role == "assistant" {
                    if let Some(message) = event.payload.get("message").cloned() {
                        if self.state.linear_active_turns.is_open(&event.task_id) {
                            tokio::spawn(
                                crate::adapters::linear::agent_runtime::notify_linear_message_activities(
                                    self.state.clone(),
                                    event.task_id.clone(),
                                    message,
                                ),
                            );
                        }
                    }
                } else if role == "user" {
                    if let Some(message) = event.payload.get("message").cloned() {
                        if self.state.linear_active_turns.is_open(&event.task_id) {
                            tokio::spawn(
                                crate::adapters::linear::agent_runtime::notify_linear_tool_results(
                                    self.state.clone(),
                                    event.task_id.clone(),
                                    message,
                                ),
                            );
                        }
                    }
                }
            }
            EventKey::Agent(AgentSubType::SessionInit) => {
                agent_handlers::handle_agent_session_init(&self.state, event).await;
            }
            EventKey::Agent(AgentSubType::Stopped) => {
                // handle_agent_stopped publishes task_turn.stopped — never
                // re-handle that downstream event or we self-amplify into a loop.
                agent_handlers::handle_agent_stopped(&self.state, event).await;
            }
            EventKey::TaskTurn(TaskTurnSubType::Stopped) => {
                // External notification is handled by the agent via `moat reply`.
            }
            EventKey::Agent(AgentSubType::Error) => {
                // handle_agent_error publishes task_turn.error; external
                // notification is handled by the agent via `moat reply`.
                agent_handlers::handle_agent_error(&self.state, event).await;
                // Also emit a persistent `error` activity on any active
                // Linear session so the failure is visible mid-flight.
                let body = event
                    .payload
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        event
                            .payload
                            .get("message")
                            .and_then(serde_json::Value::as_str)
                    })
                    .unwrap_or("Agent error")
                    .to_string();
                if self.state.linear_active_turns.is_open(&event.task_id) {
                    tokio::spawn(crate::adapters::linear::agent_runtime::notify_linear_status(
                        self.state.clone(),
                        event.task_id.clone(),
                        crate::adapters::linear::agent_api::ActivityKind::Error,
                        body,
                        false,
                    ));
                }
            }
            EventKey::TaskTurn(TaskTurnSubType::Error) => {
                // External notification is handled by the agent via `moat reply`.
            }
            EventKey::Task(TaskSubType::MetadataUpdated) => {
                agent_handlers::handle_task_metadata_updated(&self.state, event).await;
            }
            // Sandbox error — pod watcher detected a fatal crash; transcript-only
            // for the agent (auto-error reply), and a Linear `error` activity
            // for any active Linear binding.
            EventKey::Sandbox(SandboxSubType::Error) => {
                let body = event
                    .payload
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Sandbox error")
                    .to_string();
                if self.state.linear_active_turns.is_open(&event.task_id) {
                    tokio::spawn(crate::adapters::linear::agent_runtime::notify_linear_status(
                        self.state.clone(),
                        event.task_id.clone(),
                        crate::adapters::linear::agent_api::ActivityKind::Error,
                        body,
                        false,
                    ));
                }
                agent_handlers::handle_sandbox_fatal(&self.state, event).await;
            }
            // Sandbox lifecycle transitions — surface as ephemeral `action`
            // activities so each new status replaces the previous one in
            // Linear's UI (no UUIDs leak, no historical pile-up). The full
            // history still lives in Moatless and in the linked task URL.
            EventKey::Sandbox(
                status @ (SandboxSubType::Initializing
                | SandboxSubType::Ready
                | SandboxSubType::Stopped),
            ) => {
                if self.state.linear_active_turns.is_open(&event.task_id) {
                    tokio::spawn(crate::adapters::linear::agent_runtime::notify_linear_action(
                        self.state.clone(),
                        event.task_id.clone(),
                        "Sandbox".to_string(),
                        status.as_str().to_string(),
                        None,
                        true,
                    ));
                }
            }
            // Agent process started inside the sandbox — same ephemeral
            // status-marker treatment. Re-opens the Linear turn gate so a
            // manual `moat send` against a Linear-bound task re-surfaces
            // events in Linear (covers the second/third turn after the
            // agent restarts mid-session).
            EventKey::Agent(AgentSubType::Started) => {
                self.state.linear_active_turns.open(&event.task_id);
                tokio::spawn(crate::adapters::linear::agent_runtime::notify_linear_action(
                    self.state.clone(),
                    event.task_id.clone(),
                    "Agent".to_string(),
                    AgentSubType::Started.as_str().to_string(),
                    None,
                    true,
                ));
            }
            // Known SSE-only event types — forwarded to frontend but not processed here
            EventKey::Sandbox(_)
            | EventKey::Server(_)
            | EventKey::Agent(
                AgentSubType::PendingResponse
                | AgentSubType::Retrying
                | AgentSubType::Scheduled
                | AgentSubType::RecoveryStarted
                | AgentSubType::RecoveryExhausted,
            )
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
