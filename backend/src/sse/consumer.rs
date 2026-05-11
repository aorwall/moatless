//! Redis stream consumer for SSE event fan-out.
//!
//! Scans for `task-events:*` streams, creates consumer groups, and reads
//! events via `XREADGROUP`. Each event's `payload` field (raw JSON) is
//! broadcast to all connected SSE clients via the shared SSE event bus.

use redis::aio::ConnectionManager;
use redis::Value;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Consumer group name prefix. Each pod gets its own group (`sse-{hostname}`)
/// so that every pod receives every message — required for SSE fan-out where
/// any pod may have connected frontend clients.
const CONSUMER_GROUP_PREFIX: &str = "sse";

/// Redis stream consumer that fans out events to SSE clients.
pub struct RedisStreamConsumer {
    conn: ConnectionManager,
    consumer_id: String,
    /// Per-pod consumer group name (e.g. `sse-backend-abc12`).
    consumer_group: String,
    tx: broadcast::Sender<String>,
    cancel: CancellationToken,
}

impl RedisStreamConsumer {
    pub fn new(
        conn: ConnectionManager,
        consumer_id: String,
        tx: broadcast::Sender<String>,
        cancel: CancellationToken,
    ) -> Self {
        // Each pod gets its own consumer group so every pod sees every message.
        let consumer_group = format!("{CONSUMER_GROUP_PREFIX}-{consumer_id}");
        Self {
            conn,
            consumer_id,
            consumer_group,
            tx,
            cancel,
        }
    }

    /// Run the consumer loop until cancelled.
    pub async fn run(mut self) {
        info!(consumer_id = %self.consumer_id, group = %self.consumer_group, "SSE Redis consumer started");

        loop {
            if self.cancel.is_cancelled() {
                info!("SSE Redis consumer shutting down");
                break;
            }

            if let Err(e) = self.consume_cycle().await {
                if self.cancel.is_cancelled() {
                    break;
                }
                warn!(error = %e, "Redis connection lost in SSE consumer, retrying in 2s");
                tokio::select! {
                    () = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                    () = self.cancel.cancelled() => { break; }
                }
            }
        }

        info!("SSE Redis consumer stopped");
    }

    /// One consume cycle: discover streams, ensure groups, read + broadcast + ACK.
    async fn consume_cycle(&mut self) -> Result<(), redis::RedisError> {
        // Discover all task-events:* streams
        let streams = self.scan_streams().await?;

        if streams.is_empty() {
            // No streams yet — wait a bit before scanning again
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                () = self.cancel.cancelled() => {}
            }
            return Ok(());
        }

        // Ensure consumer group exists on each stream
        for stream in &streams {
            self.ensure_group(stream).await;
        }

        // Build XREADGROUP command
        // XREADGROUP GROUP sse-consumers {id} BLOCK 2000 COUNT 50 STREAMS s1 s2 ... > > ...
        let mut cmd = redis::cmd("XREADGROUP");
        cmd.arg("GROUP")
            .arg(&self.consumer_group)
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

        // Parse XREADGROUP response
        self.process_xreadgroup_response(&result, &streams).await;

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

    /// Ensure the consumer group exists on a stream (idempotent).
    async fn ensure_group(&mut self, stream: &str) {
        let result: Result<String, redis::RedisError> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream)
            .arg(&self.consumer_group)
            .arg("0") // Start from stream beginning so early events aren't missed
            .arg("MKSTREAM")
            .query_async(&mut self.conn)
            .await;

        match result {
            Ok(_) => {
                debug!(stream = %stream, group = %self.consumer_group, "Created consumer group");
            }
            Err(e) => {
                let msg = e.to_string();
                if !msg.contains("BUSYGROUP") {
                    warn!(stream = %stream, error = %e, "Failed to create consumer group");
                }
                // BUSYGROUP means group already exists — that's fine
            }
        }
    }

    /// Parse the XREADGROUP response and broadcast payloads.
    async fn process_xreadgroup_response(&mut self, value: &Value, streams: &[String]) {
        // Response format: Array of [stream_name, Array of [entry_id, [field, value, ...]]]
        let stream_results = match value {
            Value::Array(arr) => arr,
            Value::Nil => return, // Timeout with no data
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

                // Extract the `payload` field from the entry
                if let Some(payload) = extract_field(fields, "payload") {
                    // Broadcast raw JSON to all SSE subscribers
                    let _ = self.tx.send(payload);
                }

                entry_ids.push(entry_id);
            }

            // ACK all processed entries
            if !entry_ids.is_empty() {
                let ack_result: Result<i64, redis::RedisError> = {
                    let mut cmd = redis::cmd("XACK");
                    cmd.arg(&stream_name).arg(&self.consumer_group);
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

            // Find matching stream name for logging (ignore if not found)
            let _ = streams.iter().find(|s| **s == stream_name);
        }
    }
}

/// Extract a String from a Redis Value.
pub(super) fn extract_string(v: &Value) -> Option<String> {
    match v {
        Value::BulkString(bytes) => String::from_utf8(bytes.clone()).ok(),
        Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

/// Extract a named field value from a flat [key, val, key, val, ...] array.
pub(super) fn extract_field(fields: &[Value], name: &str) -> Option<String> {
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
