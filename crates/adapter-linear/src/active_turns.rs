//! Per-task gate that suppresses stray Linear activity emission after a turn
//! has terminated. Mirrors the shape of
//! `bindings::recently_sent::RecentlySent`.
//!
//! A turn opens when an inbound Linear webhook routes to a task (or an
//! `Agent(Started)` event arrives for a Linear-bound task) and closes when the
//! terminal `agent.completed`/`agent.stopped`/`agent.error` handler posts its
//! final activity; while closed, all `notify_linear_*` dispatches are dropped
//! so post-turn sandbox lifecycle events never leak into Linear's UI.
//!
//! TTL is `60min`, a safety valve (below realistic turn durations) for entries
//! stuck open after a pod crash or missed completion event.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_hours(1);

/// Process-wide map of `task_id -> open turn`.
#[derive(Debug, Default)]
pub struct ActiveLinearTurns {
    inner: Mutex<HashMap<String, OpenTurn>>,
}

#[derive(Debug)]
struct OpenTurn {
    opened_at: Instant,
    posted: HashSet<String>,
    /// Bytes of each Message's text already posted, by Message key.
    texts: HashMap<String, usize>,
}

impl ActiveLinearTurns {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark the task's Linear turn as active. Idempotent; refreshes the
    /// timestamp so the TTL safety valve resets on each turn boundary, and
    /// keeps what an already-open turn posted.
    pub fn open(&self, task_id: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            guard
                .entry(task_id.to_string())
                .and_modify(|turn| turn.opened_at = Instant::now())
                .or_insert_with(|| OpenTurn {
                    opened_at: Instant::now(),
                    posted: HashSet::new(),
                    texts: HashMap::new(),
                });
        }
    }

    /// Whether `key` may be posted now: the turn is open and has not posted
    /// it. A `true` records it as posted, so a replayed or re-projected
    /// `message.upsert` posts nothing twice.
    pub fn claim(&self, task_id: &str, key: &str) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        match guard.get_mut(task_id) {
            Some(turn) if turn.opened_at.elapsed() < TTL => turn.posted.insert(key.to_string()),
            _ => false,
        }
    }

    /// The part of a Message's text the open turn has not posted, trimmed;
    /// `None` when it is under `min_chars`, which stays unclaimed so it posts
    /// with whatever follows it. A Message's text only grows, as later blocks
    /// fold into it, so a replayed or shorter upsert claims nothing.
    pub fn claim_text(
        &self,
        task_id: &str,
        key: &str,
        text: &str,
        min_chars: usize,
    ) -> Option<String> {
        let mut guard = self.inner.lock().ok()?;
        let turn = guard
            .get_mut(task_id)
            .filter(|turn| turn.opened_at.elapsed() < TTL)?;
        let posted = turn.texts.get(key).copied().unwrap_or(0);
        let rest = text.get(posted..)?.trim();
        if rest.chars().count() < min_chars {
            return None;
        }
        turn.texts.insert(key.to_string(), text.len());
        Some(rest.to_string())
    }

    /// Mark the task's Linear turn as inactive. Absence == closed, so this is a
    /// plain `remove`.
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
        guard.retain(|_, turn| now.duration_since(turn.opened_at) < TTL);
        guard.contains_key(task_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_growing_text_is_claimed_once_per_addition() {
        let gate = ActiveLinearTurns::new();
        assert_eq!(gate.claim_text("task-1", "m", "Looking.", 4), None);
        gate.open("task-1");
        assert_eq!(
            gate.claim_text("task-1", "m", "Looking.", 4).as_deref(),
            Some("Looking.")
        );
        assert_eq!(gate.claim_text("task-1", "m", "Looking.", 4), None);
        assert_eq!(gate.claim_text("task-1", "m", "Looking.\n\nOk", 4), None);
        assert_eq!(
            gate.claim_text("task-1", "m", "Looking.\n\nOk, done.", 4)
                .as_deref(),
            Some("Ok, done.")
        );
        assert_eq!(gate.claim_text("task-1", "m", "Looking.", 4), None);
    }

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
            let stale = Instant::now()
                .checked_sub(TTL + Duration::from_secs(1))
                .expect("the process has been up longer than the TTL");
            guard.get_mut("task-1").unwrap().opened_at = stale;
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
            let stale = Instant::now()
                .checked_sub(TTL + Duration::from_secs(1))
                .expect("the process has been up longer than the TTL");
            guard.get_mut("task-1").unwrap().opened_at = stale;
        }
        assert!(!gate.is_open("task-1"));
        // The sweep also removes the entry from the map.
        let guard = gate.inner.lock().unwrap();
        assert!(!guard.contains_key("task-1"));
    }

    #[test]
    fn a_key_is_claimed_once_per_open_turn() {
        let gate = ActiveLinearTurns::new();
        assert!(!gate.claim("task-1", "call:toolu_1"));
        gate.open("task-1");
        assert!(gate.claim("task-1", "call:toolu_1"));
        assert!(!gate.claim("task-1", "call:toolu_1"));
        gate.open("task-1");
        assert!(!gate.claim("task-1", "call:toolu_1"));
        gate.close("task-1");
        gate.open("task-1");
        assert!(gate.claim("task-1", "call:toolu_1"));
    }
}
