//! Ingress dedup for sandbox-published events.
//!
//! The sandbox event outbox delivers at-least-once: on a transient failure it
//! retries the same event (same `event_id`, same `sequence`). Postgres already
//! dedupes persisted events via `ON CONFLICT (event_id) DO NOTHING`, but a
//! duplicate that reaches the gRPC handler would otherwise be re-published to
//! Redis Streams and shown twice to live SSE consumers. This short-lived
//! per-task LRU of recent event ids drops the duplicate at ingress before it is
//! ever published.

use std::time::Duration;

use moka::future::Cache;

/// How long a seen `event_id` is remembered. Comfortably longer than the
/// outbox's retry window (backoff caps at 30s) so a retried duplicate is still
/// recognised, yet short enough to bound memory.
const EVENT_ID_TTL: Duration = Duration::from_mins(5);

/// Max distinct event ids retained across all tasks. LRU eviction past this.
const MAX_EVENT_IDS: u64 = 50_000;

/// Remembers recently-seen `event_id`s so retried deliveries can be dropped.
pub struct EventDedup {
    seen: Cache<String, ()>,
    /// Last sequence observed per task, used only to log gaps/resets for
    /// observability — it never gates publishing.
    last_sequence: Cache<String, u64>,
}

impl EventDedup {
    pub fn new() -> Self {
        Self {
            seen: Cache::builder()
                .max_capacity(MAX_EVENT_IDS)
                .time_to_live(EVENT_ID_TTL)
                .build(),
            last_sequence: Cache::builder()
                .max_capacity(MAX_EVENT_IDS)
                .time_to_live(EVENT_ID_TTL)
                .build(),
        }
    }

    /// Record this delivery and report whether it is a duplicate.
    ///
    /// Returns `true` when `(task_id, event_id)` was already seen — the caller
    /// should skip publishing. An empty `event_id` (legacy sender that predates
    /// the outbox) can't be deduped and is always treated as new.
    pub async fn is_duplicate(&self, task_id: &str, event_id: &str) -> bool {
        if event_id.is_empty() {
            return false;
        }
        let key = format!("{task_id}:{event_id}");
        if self.seen.get(&key).await.is_some() {
            return true;
        }
        self.seen.insert(key, ()).await;
        false
    }

    /// Log a warning when a task's sequence skips forward (possible lost event)
    /// or moves backward (sandbox restart reset the counter). Best-effort
    /// observability only; `sequence == 0` means the sender didn't set it.
    pub async fn observe_sequence(&self, task_id: &str, sequence: u64) {
        if sequence == 0 {
            return;
        }
        if let Some(prev) = self.last_sequence.get(task_id).await {
            if sequence > prev + 1 {
                tracing::warn!(
                    task_id = %task_id,
                    prev_sequence = prev,
                    sequence,
                    "sandbox event sequence gap — a delivery may have been lost"
                );
            } else if sequence <= prev {
                tracing::debug!(
                    task_id = %task_id,
                    prev_sequence = prev,
                    sequence,
                    "sandbox event sequence did not advance (retry or restart)"
                );
            }
        }
        self.last_sequence
            .insert(task_id.to_string(), sequence)
            .await;
    }
}

impl Default for EventDedup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn first_delivery_is_not_duplicate_retry_is() {
        let dedup = EventDedup::new();
        assert!(!dedup.is_duplicate("task-1", "evt-1").await);
        // Same id again = duplicate.
        assert!(dedup.is_duplicate("task-1", "evt-1").await);
        // Different id = new.
        assert!(!dedup.is_duplicate("task-1", "evt-2").await);
        // Same id under a different task = new (per-task scoping).
        assert!(!dedup.is_duplicate("task-2", "evt-1").await);
    }

    #[tokio::test]
    async fn empty_event_id_never_deduped() {
        let dedup = EventDedup::new();
        assert!(!dedup.is_duplicate("task-1", "").await);
        assert!(!dedup.is_duplicate("task-1", "").await);
    }
}
