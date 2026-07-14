//! Liveness heartbeat for the agent actor loop.
//!
//! The agent actor is the single task that drives every agent turn. If that
//! task wedges (a processor `await` that never returns, a deadlock on a shared
//! lock) the pod keeps passing its `/health` readiness gate — `InitPhase::Ready`
//! stays set — yet no turn can ever make progress. K8s never restarts it.
//!
//! The heartbeat closes that gap: the actor stamps [`Heartbeat::beat`] on every
//! loop iteration and on a periodic tick, and the `/livez` handler checks the
//! stamp for staleness. Liveness depends ONLY on the actor being alive — never
//! on backend reachability, credentials, or disk — so a transient backend
//! outage can never trigger a restart that kills an in-flight turn.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How stale the heartbeat may get while a turn is `Running` before the actor
/// is considered wedged. Chosen well above the actor's own tick cadence so a
/// briefly busy loop never trips it.
pub const HEARTBEAT_STALE_THRESHOLD_MS: u64 = 60_000;

/// Cadence at which the actor loop bumps the heartbeat even when it is otherwise
/// blocked waiting for runtime messages. Keeps the stamp fresh while the model
/// is thinking (a long tool call emits no messages), so `Running` staleness only
/// ever means the actor task itself is stuck — not that a turn is slow.
pub const HEARTBEAT_TICK_INTERVAL: Duration = Duration::from_secs(5);

/// Current wall-clock time as milliseconds since the Unix epoch.
///
/// Falls back to `0` if the system clock is somehow before the epoch; that only
/// makes the heartbeat look maximally stale, which fails safe (the handler still
/// checks the command channel before declaring the actor dead).
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Shared, cheaply-clonable liveness stamp bumped by the agent actor loop.
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
    /// actor is never momentarily considered stale.
    pub fn new() -> Self {
        Self {
            last_beat_ms: Arc::new(AtomicU64::new(now_millis())),
        }
    }

    /// Record that the actor loop is alive right now.
    pub fn beat(&self) {
        self.last_beat_ms.store(now_millis(), Ordering::Relaxed);
    }

    /// Epoch-millis of the most recent beat.
    pub fn last_beat_ms(&self) -> u64 {
        self.last_beat_ms.load(Ordering::Relaxed)
    }
}

/// Pure liveness decision for the agent actor.
///
/// - A closed command channel means the actor loop has exited entirely — no turn
///   can ever run again, so the sandbox is unhealthy regardless of turn state.
/// - While a turn is `Running` the actor loop must keep beating; a stale stamp
///   means the loop is wedged mid-turn.
/// - When idle the loop legitimately blocks on the command channel, so an open
///   channel is a sufficient liveness signal on its own.
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
