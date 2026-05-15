//! Per-task gate that suppresses stray Linear activity emission after a turn
//! has terminated. Mirrors the shape of `bindings::recently_sent::RecentlySent`.
//!
//! A turn "opens" when an inbound Linear webhook routes to a task (or when an
//! `Agent(Started)` event arrives for a Linear-bound task) and "closes" when
//! the terminal `agent.completed` / `agent.stopped` / `agent.error` handler
//! has finished posting its final activity. While the gate is closed, all
//! event-consumer dispatches to `notify_linear_*` are dropped so post-turn
//! sandbox lifecycle events (`Sandbox(Stopped)`, late tool_results, etc.)
//! never leak into Linear's UI.
//!
//! TTL is `60min` — a safety valve for entries that get stuck open after a
//! pod crash or missed completion event. Below realistic turn durations.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(60 * 60);

/// Process-wide map of `task_id -> opened-at instant`.
#[derive(Debug, Default)]
pub struct ActiveLinearTurns {
    inner: Mutex<HashMap<String, Instant>>,
}

impl ActiveLinearTurns {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark the task's Linear turn as active. Idempotent; refreshes the
    /// timestamp so the TTL safety valve resets on each turn boundary.
    pub fn open(&self, task_id: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(task_id.to_string(), Instant::now());
        }
    }

    /// Mark the task's Linear turn as inactive. Absence == closed, so this is
    /// a plain `remove`.
    pub fn close(&self, task_id: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.remove(task_id);
        }
    }

    /// Returns true when the task has an open turn within the TTL window.
    /// Sweeps expired entries opportunistically on each call.
    pub fn is_open(&self, task_id: &str) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };

        let now = Instant::now();
        guard.retain(|_, ts| now.duration_since(*ts) < TTL);

        match guard.get(task_id) {
            Some(ts) => now.duration_since(*ts) < TTL,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_then_is_open() {
        let gate = ActiveLinearTurns::new();
        gate.open("task-1");
        assert!(gate.is_open("task-1"));
    }

    #[test]
    fn closed_by_default() {
        let gate = ActiveLinearTurns::new();
        assert!(!gate.is_open("task-1"));
    }

    #[test]
    fn close_removes_entry() {
        let gate = ActiveLinearTurns::new();
        gate.open("task-1");
        gate.close("task-1");
        assert!(!gate.is_open("task-1"));
    }

    #[test]
    fn open_is_per_task() {
        let gate = ActiveLinearTurns::new();
        gate.open("task-1");
        assert!(gate.is_open("task-1"));
        assert!(!gate.is_open("task-2"));
    }

    #[test]
    fn open_is_idempotent_and_refreshes_ttl() {
        // Force a stale timestamp, then re-open and confirm the gate is open.
        let gate = ActiveLinearTurns::new();
        gate.open("task-1");
        {
            let mut guard = gate.inner.lock().unwrap();
            let stale = Instant::now() - (TTL + Duration::from_secs(1));
            guard.insert("task-1".to_string(), stale);
        }
        // Stale: TTL sweep drops it on is_open().
        assert!(!gate.is_open("task-1"));
        // Re-open refreshes the timestamp.
        gate.open("task-1");
        assert!(gate.is_open("task-1"));
    }

    #[test]
    fn ttl_sweep_drops_expired() {
        let gate = ActiveLinearTurns::new();
        gate.open("task-1");
        {
            let mut guard = gate.inner.lock().unwrap();
            let stale = Instant::now() - (TTL + Duration::from_secs(1));
            guard.insert("task-1".to_string(), stale);
        }
        assert!(!gate.is_open("task-1"));
        // The sweep also removes the entry from the map.
        let guard = gate.inner.lock().unwrap();
        assert!(!guard.contains_key("task-1"));
    }
}
