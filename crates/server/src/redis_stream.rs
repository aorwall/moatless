//! The Redis stream plumbing the SSE and event-handler consumers share.
//!
//! Both read the same `task-events:*` streams through `XREADGROUP` under their
//! own consumer group and differ only in what they do with an entry's
//! `payload` field. [`StreamConsumer`] owns stream discovery, group creation,
//! pending-entry recovery, ACK and the reconnect loop; a [`StreamHandler`]
//! impl supplies the per-entry work and whatever else its consumer needs.

use async_trait::async_trait;
use redis::Value;
use redis::aio::ConnectionManager;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Key pattern every Task's event stream matches.
const STREAM_PATTERN: &str = "task-events:*";

/// Where a newly created consumer group starts reading.
#[derive(Clone, Copy, Debug)]
pub(crate) enum GroupStart {
    /// Only entries added after the group exists (`$`).
    New,
    /// Every entry still in the stream (`0`).
    Beginning,
}

impl GroupStart {
    fn id(self) -> &'static str {
        match self {
            Self::New => "$",
            Self::Beginning => "0",
        }
    }
}

/// Which slice of a stream an `XREADGROUP` should read.
#[derive(Clone, Copy)]
enum ReadMode {
    /// New, never-delivered entries (`>`), blocking briefly for arrivals.
    New,
    /// This consumer's pending (delivered-but-unacked) entries (`0`).
    Pending,
}

impl ReadMode {
    fn id(self) -> &'static str {
        match self {
            Self::New => ">",
            Self::Pending => "0",
        }
    }
}

/// The per-entry work a consumer does; [`StreamConsumer`] owns everything else.
#[async_trait]
pub(crate) trait StreamHandler: Send {
    /// Act on one entry's `payload` field. `entry_id` identifies it in the
    /// stream, for a handler that rejects the payload and wants to say which.
    ///
    /// The entry is ACKed whatever this does, so a payload the handler rejects
    /// is never redelivered.
    async fn handle_payload(&mut self, entry_id: &str, payload: String);

    /// Runs each cycle after group creation and before any read, with the
    /// streams just discovered. Shares the consumer's connection.
    async fn before_read(&mut self, _conn: &mut ConnectionManager, _streams: &[String]) {}

    /// Runs once, when the loop leaves on cancellation.
    async fn on_shutdown(&mut self, _conn: &mut ConnectionManager) {}
}

/// Drives one consumer group across every `task-events:*` stream.
pub(crate) struct StreamConsumer<H> {
    conn: ConnectionManager,
    consumer_id: String,
    consumer_group: String,
    group_start: GroupStart,
    /// Names the consumer in the log lines both of them emit.
    kind: &'static str,
    handler: H,
    cancel: CancellationToken,
    /// When set, the next cycle drains the Pending Entries List
    /// (delivered-but-not-ACKed) before reading new entries.
    ///
    /// Set on startup and after every connection error, so an entry lost
    /// mid-cycle during Redis flapping is recovered rather than orphaned.
    recover_pending: bool,
}

impl<H: StreamHandler> StreamConsumer<H> {
    pub(crate) fn new(
        conn: ConnectionManager,
        consumer_id: String,
        consumer_group: String,
        group_start: GroupStart,
        kind: &'static str,
        handler: H,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            conn,
            consumer_id,
            consumer_group,
            group_start,
            kind,
            handler,
            cancel,
            recover_pending: true,
        }
    }

    /// Run the consumer loop until cancelled.
    pub(crate) async fn run(mut self) {
        info!(
            consumer_id = %self.consumer_id,
            group = %self.consumer_group,
            kind = self.kind,
            "Redis stream consumer started"
        );

        let mut failures: u32 = 0;
        loop {
            if self.cancel.is_cancelled() {
                info!(kind = self.kind, "Redis stream consumer shutting down");
                self.handler.on_shutdown(&mut self.conn).await;
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
                    let delay = moatless_core::events::reconnect_backoff_delay(failures);
                    warn!(
                        kind = self.kind,
                        error = %e,
                        failures,
                        delay_ms = delay.as_millis() as u64,
                        "Redis connection lost, backing off"
                    );
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {}
                        () = self.cancel.cancelled() => { break; }
                    }
                }
            }
        }

        info!(kind = self.kind, "Redis stream consumer stopped");
    }

    /// One cycle: discover streams, ensure groups, recover pending, then read
    /// + handle + ACK new entries.
    async fn consume_cycle(&mut self) -> Result<(), redis::RedisError> {
        let streams = scan_streams(&mut self.conn).await?;

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

        self.handler.before_read(&mut self.conn, &streams).await;

        if self.recover_pending {
            self.recover_pending_entries(&streams).await?;
            self.recover_pending = false;
        }

        let result = self.read_group(&streams, ReadMode::New).await?;
        self.process_response(&result).await;

        Ok(())
    }

    /// Drain this consumer's Pending Entries List across all streams, handling
    /// and ACKing until empty.
    ///
    /// Reading with id `0` returns the PEL from the start and each pass ACKs
    /// what it handled, so successive reads shrink to nothing. The 100-pass cap
    /// bounds a poison entry, which is ACKed too.
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
                kind = self.kind,
                group = %self.consumer_group,
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
        for _ in streams {
            cmd.arg(mode.id());
        }

        cmd.query_async(&mut self.conn).await
    }

    /// Ensure the consumer group exists on a stream (idempotent).
    async fn ensure_group(&mut self, stream: &str) {
        let result: Result<String, redis::RedisError> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream)
            .arg(&self.consumer_group)
            .arg(self.group_start.id())
            .arg("MKSTREAM")
            .query_async(&mut self.conn)
            .await;

        match result {
            Ok(_) => {
                debug!(stream = %stream, group = %self.consumer_group, "Created consumer group");
            }
            Err(e) => {
                let msg = e.to_string();
                // BUSYGROUP means the group already exists, which is the point.
                if !msg.contains("BUSYGROUP") {
                    warn!(stream = %stream, error = %e, "Failed to create consumer group");
                }
            }
        }
    }

    /// Parse an `XREADGROUP` response, hand each `payload` to the handler, and
    /// ACK. Returns the number of entries ACKed, letting the PEL drain loop
    /// detect an empty read.
    async fn process_response(&mut self, value: &Value) -> usize {
        // Response format: array of [stream_name, array of [entry_id, [field, value, ...]]]
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
            let Some((stream_name, entries)) = split_pair(stream_result) else {
                continue;
            };

            let mut entry_ids = Vec::new();

            for entry in entries {
                let Some((entry_id, fields)) = split_pair(entry) else {
                    continue;
                };

                if let Some(payload) = extract_field(fields, "payload") {
                    self.handler.handle_payload(&entry_id, payload).await;
                }

                entry_ids.push(entry_id);
            }

            if !entry_ids.is_empty() {
                processed += entry_ids.len();
                self.ack(&stream_name, &entry_ids).await;
            }
        }

        processed
    }

    /// ACK a batch of entry ids on one stream.
    async fn ack(&mut self, stream_name: &str, entry_ids: &[String]) {
        let result: Result<i64, redis::RedisError> = {
            let mut cmd = redis::cmd("XACK");
            cmd.arg(stream_name).arg(&self.consumer_group);
            for id in entry_ids {
                cmd.arg(id);
            }
            cmd.query_async(&mut self.conn).await
        };

        if let Err(e) = result {
            warn!(
                stream = %stream_name,
                count = entry_ids.len(),
                error = %e,
                "Failed to ACK entries"
            );
        }
    }
}

/// SCAN for keys matching [`STREAM_PATTERN`].
///
/// Free rather than a method so a handler can reach it on the shared
/// connection it is handed.
pub(crate) async fn scan_streams(
    conn: &mut ConnectionManager,
) -> Result<Vec<String>, redis::RedisError> {
    let mut streams = Vec::new();
    let mut cursor = 0_u64;

    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(STREAM_PATTERN)
            .arg("COUNT")
            .arg(100)
            .query_async(conn)
            .await?;

        streams.extend(keys);
        cursor = next_cursor;

        if cursor == 0 {
            break;
        }
    }

    Ok(streams)
}

/// Split a `[name, array]` reply pair, which is how both a stream result and a
/// stream entry arrive.
fn split_pair(value: &Value) -> Option<(String, &Vec<Value>)> {
    let Value::Array(pair) = value else {
        return None;
    };
    if pair.len() != 2 {
        return None;
    }
    let name = extract_string(&pair[0]).unwrap_or_default();
    match &pair[1] {
        Value::Array(items) => Some((name, items)),
        _ => None,
    }
}

/// Extract a String from a Redis Value.
pub(crate) fn extract_string(v: &Value) -> Option<String> {
    match v {
        Value::BulkString(bytes) => String::from_utf8(bytes.clone()).ok(),
        Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

/// Extract a named field value from a flat [key, val, key, val, ...] array.
pub(crate) fn extract_field(fields: &[Value], name: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::{extract_field, extract_string, split_pair};
    use redis::Value;

    fn bulk(s: &str) -> Value {
        Value::BulkString(s.as_bytes().to_vec())
    }

    #[test]
    fn split_pair_reads_a_stream_result() {
        let entry = Value::Array(vec![bulk("1-0"), Value::Array(vec![bulk("payload")])]);
        let result = Value::Array(vec![
            bulk("task-events:abc"),
            Value::Array(vec![entry.clone()]),
        ]);
        let (name, entries) = split_pair(&result).expect("well-formed pair");
        assert_eq!(name, "task-events:abc");
        assert_eq!(entries, &vec![entry]);
    }

    #[test]
    fn split_pair_rejects_malformed_replies() {
        // Wrong arity, and a second element that is not an array of entries.
        assert!(split_pair(&Value::Array(vec![bulk("only-one")])).is_none());
        assert!(split_pair(&Value::Array(vec![bulk("a"), bulk("b"), bulk("c")])).is_none());
        assert!(split_pair(&Value::Array(vec![bulk("a"), bulk("not-an-array")])).is_none());
        assert!(split_pair(&Value::Nil).is_none());
    }

    #[test]
    fn extract_field_reads_a_flat_key_value_list() {
        let fields = vec![bulk("event_id"), bulk("e1"), bulk("payload"), bulk("{}")];
        assert_eq!(extract_field(&fields, "payload").as_deref(), Some("{}"));
        assert_eq!(extract_field(&fields, "missing"), None);
    }

    #[test]
    fn extract_field_stops_on_a_dangling_key() {
        // An odd-length list leaves the last key without a value.
        let fields = vec![bulk("payload"), bulk("{}"), bulk("dangling")];
        assert_eq!(extract_field(&fields, "payload").as_deref(), Some("{}"));
        assert_eq!(extract_field(&fields, "dangling"), None);
    }

    #[test]
    fn extract_string_reads_both_string_encodings_only() {
        assert_eq!(extract_string(&bulk("x")).as_deref(), Some("x"));
        assert_eq!(
            extract_string(&Value::SimpleString("y".to_string())).as_deref(),
            Some("y")
        );
        assert_eq!(extract_string(&Value::Int(3)), None);
    }
}
