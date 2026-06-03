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
    /// When set, the next cycle first re-broadcasts this consumer's pending
    /// (delivered-but-unacked) entries before reading new ones, so a payload
    /// dropped mid-cycle during Redis flapping still reaches SSE clients.
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
            recover_pending: true,
        }
    }

    /// Run the consumer loop until cancelled.
    pub async fn run(mut self) {
        info!(consumer_id = %self.consumer_id, group = %self.consumer_group, "SSE Redis consumer started");

        let mut failures: u32 = 0;
        loop {
            if self.cancel.is_cancelled() {
                info!("SSE Redis consumer shutting down");
                break;
            }

            match self.consume_cycle().await {
                Ok(()) => failures = 0,
                Err(e) => {
                    if self.cancel.is_cancelled() {
                        break;
                    }
                    failures = failures.saturating_add(1);
                    self.recover_pending = true;
                    let delay = crate::events::reconnect_backoff_delay(failures);
                    warn!(
                        error = %e,
                        failures,
                        delay_ms = delay.as_millis() as u64,
                        "Redis connection lost in SSE consumer, backing off"
                    );
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {}
                        () = self.cancel.cancelled() => { break; }
                    }
                }
            }
        }

        info!("SSE Redis consumer stopped");
    }

    /// One consume cycle: discover streams, ensure groups, re-broadcast pending,
    /// then read + broadcast + ACK new entries.
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

        // Re-broadcast entries delivered to this group but not ACKed before a
        // disconnect, so SSE clients don't miss a payload lost mid-cycle.
        if self.recover_pending {
            self.recover_pending_entries(&streams).await?;
            self.recover_pending = false;
        }

        let result = self.read_group(&streams, ReadMode::New).await?;
        self.process_xreadgroup_response(&result, &streams).await;

        Ok(())
    }

    /// Drain this consumer's Pending Entries List across all streams,
    /// re-broadcasting and ACKing until empty. Reading with id `0` returns the
    /// PEL from the start; each pass ACKs what it broadcasts, so successive
    /// reads shrink to nothing. Capped to bound the loop.
    async fn recover_pending_entries(
        &mut self,
        streams: &[String],
    ) -> Result<(), redis::RedisError> {
        let mut total = 0usize;
        for _ in 0..100 {
            let result = self.read_group(streams, ReadMode::Pending).await?;
            let count = self.process_xreadgroup_response(&result, streams).await;
            total += count;
            if count == 0 {
                break;
            }
        }
        if total > 0 {
            info!(
                group = %self.consumer_group,
                recovered = total,
                "Re-broadcast pending Redis entries after reconnect"
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
        cmd.arg("GROUP")
            .arg(&self.consumer_group)
            .arg(&self.consumer_id);
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

    /// Parse the XREADGROUP response and broadcast payloads. Returns the number
    /// of entries ACKed, letting the PEL drain loop detect an empty read.
    async fn process_xreadgroup_response(&mut self, value: &Value, streams: &[String]) -> usize {
        // Response format: Array of [stream_name, Array of [entry_id, [field, value, ...]]]
        let stream_results = match value {
            Value::Array(arr) => arr,
            Value::Nil => return 0, // Timeout with no data
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

                // Extract the `payload` field from the entry
                if let Some(payload) = extract_field(fields, "payload") {
                    // Broadcast raw JSON to all SSE subscribers
                    let _ = self.tx.send(payload);
                }

                entry_ids.push(entry_id);
            }

            // ACK all processed entries
            if !entry_ids.is_empty() {
                processed += entry_ids.len();
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

        processed
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
