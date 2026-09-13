//! Redis stream consumer for SSE event fan-out.
//!
//! Broadcasts each event's `payload` field (raw JSON) to all connected SSE
//! clients via the shared SSE event bus. The stream discovery, group and ACK
//! plumbing it shares with the event-handler consumer lives in
//! [`crate::redis_stream`]; what stays here is the broadcast and the per-pod
//! group's own lifecycle.

use async_trait::async_trait;
use redis::Value;
use redis::aio::ConnectionManager;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::redis_stream::{
    GroupStart, StreamConsumer, StreamHandler, extract_field, extract_string, scan_streams,
};

/// Consumer group name prefix. Each pod gets its own group (`sse-{hostname}`)
/// so that every pod receives every message — required for SSE fan-out where
/// any pod may have connected frontend clients.
const CONSUMER_GROUP_PREFIX: &str = "sse";

/// How long every consumer in a group must sit idle before the group counts as
/// orphaned. A live pod re-reads on each cycle (`BLOCK 2000`), so idleness on
/// this scale means the pod that owned the group is gone.
const GROUP_ORPHAN_IDLE_MS: i64 = 60 * 60 * 1000;

/// Interval between orphan sweeps.
///
/// Per-pod groups are created on *every* stream, so a pod that dies without a
/// clean shutdown strands one group per stream; nothing else removes them, and
/// Redis charges memory per group and PEL entry, so they'd accumulate forever
/// without this sweep.
const GROUP_REAP_INTERVAL: std::time::Duration = std::time::Duration::from_mins(30);

/// Broadcasts stream payloads to SSE clients and keeps this pod's groups tidy.
struct SseHandler {
    tx: broadcast::Sender<String>,
    /// Per-pod consumer group name (e.g. `sse-backend-abc12`).
    consumer_group: String,
    /// When the next orphaned-group sweep is due.
    next_reap: std::time::Instant,
}

#[async_trait]
impl StreamHandler for SseHandler {
    async fn handle_payload(&mut self, _entry_id: &str, payload: String) {
        // Relay to clients with `agent.message` reduced to its metadata
        // skeleton (the agent.message skeleton): content reaches the Chat once
        // via the derived `message.upsert`, never a second time as the raw
        // source event. Non-agent-message events relay untouched.
        let relayed =
            crate::event_handler::skeleton::slim_relayed_event(&payload).unwrap_or(payload);
        let _ = self.tx.send(relayed);
    }

    /// Drop groups left behind by pods that died without a clean shutdown.
    async fn before_read(&mut self, conn: &mut ConnectionManager, streams: &[String]) {
        if std::time::Instant::now() >= self.next_reap {
            self.reap_orphaned_groups(conn, streams).await;
            self.next_reap = std::time::Instant::now() + GROUP_REAP_INTERVAL;
        }
    }

    async fn on_shutdown(&mut self, conn: &mut ConnectionManager) {
        self.destroy_own_groups(conn).await;
    }
}

impl SseHandler {
    /// Destroy this pod's own groups across all streams.
    ///
    /// The clean-shutdown counterpart to group creation; a pod killed outright
    /// never runs this, which is what [`Self::reap_orphaned_groups`] is for.
    async fn destroy_own_groups(&mut self, conn: &mut ConnectionManager) {
        let streams = match scan_streams(conn).await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "Could not scan streams to drop SSE consumer groups");
                return;
            }
        };

        let mut destroyed = 0usize;
        for stream in &streams {
            let result: Result<i64, redis::RedisError> = redis::cmd("XGROUP")
                .arg("DESTROY")
                .arg(stream)
                .arg(&self.consumer_group)
                .query_async(&mut *conn)
                .await;
            match result {
                Ok(n) => destroyed += usize::try_from(n).unwrap_or(0),
                Err(e) => {
                    debug!(stream = %stream, error = %e, "Failed to destroy SSE consumer group");
                }
            }
        }

        info!(
            group = %self.consumer_group,
            destroyed,
            "Dropped SSE consumer groups on shutdown"
        );
    }

    /// Destroy `sse-*` groups whose consumers have all gone idle past
    /// [`GROUP_ORPHAN_IDLE_MS`] — pods that died without a clean shutdown.
    ///
    /// Never touches this pod's own group, nor `event-handler` (which owns
    /// persistence to Postgres); a live pod's group is re-read every couple of
    /// seconds, so it can't be mistaken for an orphan.
    async fn reap_orphaned_groups(&mut self, conn: &mut ConnectionManager, streams: &[String]) {
        let mut destroyed = 0usize;

        for stream in streams {
            let groups: Value = match redis::cmd("XINFO")
                .arg("GROUPS")
                .arg(stream)
                .query_async(&mut *conn)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    debug!(stream = %stream, error = %e, "XINFO GROUPS failed during reap");
                    continue;
                }
            };

            for name in orphan_candidates(&groups, &self.consumer_group) {
                if !group_is_idle(conn, stream, &name).await {
                    continue;
                }
                let result: Result<i64, redis::RedisError> = redis::cmd("XGROUP")
                    .arg("DESTROY")
                    .arg(stream)
                    .arg(&name)
                    .query_async(&mut *conn)
                    .await;
                match result {
                    Ok(n) => destroyed += usize::try_from(n).unwrap_or(0),
                    Err(e) => {
                        debug!(stream = %stream, group = %name, error = %e, "Failed to destroy orphaned group");
                    }
                }
            }
        }

        if destroyed > 0 {
            info!(
                destroyed,
                streams = streams.len(),
                "Reaped orphaned SSE consumer groups"
            );
        }
    }
}

/// True when every consumer in `group` has been idle past the orphan
/// threshold. A group with no consumers at all is not reaped — it has no
/// idleness to judge and costs almost nothing.
async fn group_is_idle(conn: &mut ConnectionManager, stream: &str, group: &str) -> bool {
    let consumers: Value = match redis::cmd("XINFO")
        .arg("CONSUMERS")
        .arg(stream)
        .arg(group)
        .query_async(conn)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            debug!(stream = %stream, group = %group, error = %e, "XINFO CONSUMERS failed");
            return false;
        }
    };

    let Value::Array(entries) = consumers else {
        return false;
    };
    if entries.is_empty() {
        return false;
    }

    entries.iter().all(|entry| match entry {
        Value::Array(fields) => {
            extract_i64(fields, "idle").is_some_and(|idle| idle > GROUP_ORPHAN_IDLE_MS)
        }
        _ => false,
    })
}

/// Redis stream consumer that fans out events to SSE clients.
pub struct RedisStreamConsumer(StreamConsumer<SseHandler>);

impl RedisStreamConsumer {
    pub fn new(
        conn: ConnectionManager,
        consumer_id: String,
        tx: broadcast::Sender<String>,
        cancel: CancellationToken,
    ) -> Self {
        // Each pod gets its own consumer group so every pod sees every message.
        let consumer_group = format!("{CONSUMER_GROUP_PREFIX}-{consumer_id}");
        let handler = SseHandler {
            tx,
            consumer_group: consumer_group.clone(),
            // Sweep once on startup: this pod replaces one that may well have
            // died without cleaning up after itself.
            next_reap: std::time::Instant::now(),
        };
        Self(StreamConsumer::new(
            conn,
            consumer_id,
            consumer_group,
            // From the stream's beginning, so events published before this pod
            // came up still reach the clients it is about to serve.
            GroupStart::Beginning,
            "sse",
            handler,
            cancel,
        ))
    }

    /// Run the consumer loop until cancelled.
    pub async fn run(self) {
        self.0.run().await;
    }
}

/// Extract a named integer field from a flat [key, val, ...] array. `XINFO`
/// returns numbers as `Value::Int`, which [`extract_string`] can't read.
fn extract_i64(fields: &[Value], name: &str) -> Option<i64> {
    let mut iter = fields.iter();
    while let Some(key) = iter.next() {
        let val = iter.next()?;
        if extract_string(key).as_deref() == Some(name) {
            return match val {
                Value::Int(n) => Some(*n),
                _ => extract_string(val).and_then(|s| s.parse().ok()),
            };
        }
    }
    None
}

/// Names of `sse-*` groups in an `XINFO GROUPS` reply, excluding `own_group`.
///
/// Only `sse-` groups are candidates: `event-handler` is a single shared group
/// that persists events to Postgres and must never be reaped.
fn orphan_candidates(groups: &Value, own_group: &str) -> Vec<String> {
    let Value::Array(entries) = groups else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| match entry {
            Value::Array(fields) => extract_field(fields, "name"),
            _ => None,
        })
        .filter(|name| name.starts_with(&format!("{CONSUMER_GROUP_PREFIX}-")) && name != own_group)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{extract_i64, orphan_candidates};
    use redis::Value;

    /// One `XINFO GROUPS` entry: a flat [key, val, ...] array.
    fn group_entry(name: &str) -> Value {
        Value::Array(vec![
            Value::BulkString(b"name".to_vec()),
            Value::BulkString(name.as_bytes().to_vec()),
            Value::BulkString(b"consumers".to_vec()),
            Value::Int(1),
        ])
    }

    #[test]
    fn own_group_is_never_a_candidate() {
        let groups = Value::Array(vec![group_entry("sse-pod-a"), group_entry("sse-pod-b")]);
        let candidates = orphan_candidates(&groups, "sse-pod-a");
        assert_eq!(candidates, vec!["sse-pod-b".to_string()]);
    }

    #[test]
    fn event_handler_group_is_never_a_candidate() {
        // Reaping this group would break persistence to Postgres.
        let groups = Value::Array(vec![group_entry("event-handler"), group_entry("sse-pod-b")]);
        let candidates = orphan_candidates(&groups, "sse-pod-a");
        assert_eq!(candidates, vec!["sse-pod-b".to_string()]);
    }

    #[test]
    fn non_sse_groups_are_ignored() {
        let groups = Value::Array(vec![group_entry("some-other-consumer")]);
        assert!(orphan_candidates(&groups, "sse-pod-a").is_empty());
    }

    #[test]
    fn extract_i64_reads_int_and_string_encodings() {
        let fields = vec![
            Value::BulkString(b"idle".to_vec()),
            Value::Int(4200),
            Value::BulkString(b"pending".to_vec()),
            Value::BulkString(b"7".to_vec()),
        ];
        assert_eq!(extract_i64(&fields, "idle"), Some(4200));
        assert_eq!(extract_i64(&fields, "pending"), Some(7));
        assert_eq!(extract_i64(&fields, "missing"), None);
    }
}
