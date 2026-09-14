//! Liveness heartbeat for the agent lifecycle task.
//!
//! If the coordinator service wedges (a deadlock, an `await` that never
//! returns), the pod keeps passing `/health` and K8s never restarts it, yet no
//! turn can progress.
//!
//! The lifecycle task stamps [`Heartbeat::beat`] each loop iteration and on a
//! tick; `/livez` checks staleness ONLY against that stamp, never against
//! backend reachability, credentials, or disk.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::Utc;

/// How stale the heartbeat may get while a turn is `Running` before the service
/// is considered wedged. Chosen well above the service's own tick cadence so a
/// briefly busy loop never trips it.
pub const HEARTBEAT_STALE_THRESHOLD_MS: u64 = 60_000;

/// Cadence at which the lifecycle task bumps the heartbeat even while blocked
/// waiting for runtime messages.
///
/// Keeps the stamp fresh while the model is thinking, so `Running` staleness
/// means the lifecycle task itself is stuck, never that a turn is slow.
pub const HEARTBEAT_TICK_INTERVAL: Duration = Duration::from_secs(5);

/// Current wall-clock time as milliseconds since the Unix epoch.
///
/// Falls back to `0` before the epoch, which only makes the heartbeat look
/// maximally stale — fails safe, since the handler still checks the command
/// channel first.
pub fn now_millis() -> u64 {
    u64::try_from(Utc::now().timestamp_millis()).unwrap_or(0)
}

/// Shared, cheaply-clonable liveness stamp bumped by the agent lifecycle task.
#[derive(Clone)]
pub struct Heartbeat {
    last_beat_ms: Arc<AtomicU64>,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self::new()
    }
}

impl Heartbeat {
    /// Create a heartbeat stamped with the current time so a freshly-spawned
    /// service is never momentarily considered stale.
    pub fn new() -> Self {
        Self {
            last_beat_ms: Arc::new(AtomicU64::new(now_millis())),
        }
    }

    /// Record that the lifecycle task is alive right now.
    pub fn beat(&self) {
        self.last_beat_ms.store(now_millis(), Ordering::Relaxed);
    }

    /// Epoch-millis of the most recent beat.
    pub fn last_beat_ms(&self) -> u64 {
        self.last_beat_ms.load(Ordering::Relaxed)
    }
}

/// Pure liveness decision for the agent lifecycle task.
///
/// A closed command channel is always dead; an open one is alive unless a
/// `Running` turn's heartbeat has gone stale, meaning the loop is wedged
/// mid-turn.
pub fn actor_is_alive(
    now_ms: u64,
    last_beat_ms: u64,
    turn_running: bool,
    command_channel_open: bool,
    stale_threshold_ms: u64,
) -> bool {
    if !command_channel_open {
        return false;
    }
    if turn_running {
        now_ms.saturating_sub(last_beat_ms) <= stale_threshold_ms
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = HEARTBEAT_STALE_THRESHOLD_MS;

    #[test]
    fn closed_channel_is_always_dead() {
        // Even a perfectly fresh beat can't save a closed actor channel.
        assert!(!actor_is_alive(1_000, 1_000, false, false, T));
        assert!(!actor_is_alive(1_000, 1_000, true, false, T));
    }

    #[test]
    fn running_with_fresh_beat_is_alive() {
        let now = 100_000;
        assert!(actor_is_alive(now, now - 1_000, true, true, T));
    }

    #[test]
    fn running_with_stale_beat_is_dead() {
        let now = 100_000;
        // Beat is older than the threshold while a turn is running → wedged.
        assert!(!actor_is_alive(now, now - (T + 1), true, true, T));
    }

    #[test]
    fn running_exactly_at_threshold_is_alive() {
        let now = 100_000;
        // Boundary is inclusive: exactly `threshold` old still counts as alive.
        assert!(actor_is_alive(now, now - T, true, true, T));
    }

    #[test]
    fn idle_ignores_staleness_when_channel_open() {
        let now = 100_000;
        // Idle loop blocks on the command channel and doesn't beat — a very old
        // stamp is expected and healthy as long as the channel is open.
        assert!(actor_is_alive(now, 0, false, true, T));
    }

    #[test]
    fn future_beat_does_not_underflow() {
        // Clock skew that puts the last beat ahead of `now` must not panic or
        // wrap; saturating subtraction yields 0 → alive.
        assert!(actor_is_alive(1_000, 5_000, true, true, T));
    }

    #[test]
    fn new_heartbeat_is_fresh() {
        let hb = Heartbeat::new();
        let now = now_millis();
        assert!(now.saturating_sub(hb.last_beat_ms()) <= 1_000);
    }

    #[test]
    fn beat_advances_stamp() {
        let hb = Heartbeat::new();
        hb.last_beat_ms.store(0, Ordering::Relaxed);
        hb.beat();
        assert!(hb.last_beat_ms() > 0);
    }
}
