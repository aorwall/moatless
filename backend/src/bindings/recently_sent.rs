//! Short-lived in-memory cache of `(task_id, external_event_id)` pairs we
//! have just sent outbound, so the inbound webhook for the same id can be
//! dropped before it reaches the per-row DB tag — closing the race where the
//! webhook arrives before the outbound `set_external_event_id` UPDATE
//! commits.
//!
//! Lossy by design: if the backend restarts between outbound and webhook the
//! DB tag still catches it, and the per-thread `should_filter_self_echo`
//! guard catches anything that slips past both.
//!
//! TTL is `120s` — comfortably above realistic webhook delivery latency and
//! well below realistic external-id reuse windows.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_mins(2);

/// Process-wide cache of recent outbound external ids.
#[derive(Debug, Default)]
pub struct RecentlySent {
    inner: Mutex<HashMap<String, Instant>>,
}

impl RecentlySent {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that we just sent an outbound that produced `ext_id` for `task_id`.
    /// Subsequent inbound webhooks carrying the same id within the TTL window
    /// will be deduped via [`Self::contains`].
    pub fn note(&self, task_id: &str, ext_id: &str) {
        let key = key(task_id, ext_id);
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(key, Instant::now());
        }
    }

    /// Check whether `(task_id, ext_id)` was recorded within the TTL window.
    /// Sweeps expired entries opportunistically on each call.
    pub fn contains(&self, task_id: &str, ext_id: &str) -> bool {
        let key = key(task_id, ext_id);
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };

        let now = Instant::now();
        guard.retain(|_, ts| now.duration_since(*ts) < TTL);

        match guard.get(&key) {
            Some(ts) => now.duration_since(*ts) < TTL,
            None => false,
        }
    }
}

fn key(task_id: &str, ext_id: &str) -> String {
    format!("{task_id}::{ext_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_then_contains_within_ttl() {
        let cache = RecentlySent::new();
        cache.note("task-1", "github:ic:42");
        assert!(cache.contains("task-1", "github:ic:42"));
    }

    #[test]
    fn contains_misses_for_other_task() {
        let cache = RecentlySent::new();
        cache.note("task-1", "github:ic:42");
        assert!(!cache.contains("task-2", "github:ic:42"));
        assert!(!cache.contains("task-1", "github:ic:43"));
    }

    #[test]
    fn manual_sweep_drops_expired() {
        // We can't reliably advance time in unit tests without injecting a
        // clock. Instead, smoke-test the sweep path by inserting then forcing
        // a stale timestamp.
        let cache = RecentlySent::new();
        cache.note("task-1", "github:ic:42");
        {
            let mut guard = cache.inner.lock().unwrap();
            let key = key("task-1", "github:ic:42");
            let stale = Instant::now() - (TTL + Duration::from_secs(1));
            guard.insert(key, stale);
        }
        assert!(!cache.contains("task-1", "github:ic:42"));
    }
}
