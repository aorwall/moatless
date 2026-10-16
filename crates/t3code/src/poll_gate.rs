//! A per-replica bound on the database work T3 subscriptions do, and the
//! jitter that keeps the loops still on a timer out of phase.
//!
//! [`crate::change_watch::ChangePoller`] wakes every subscription on one head
//! move at once, and a reconnect wave opens them all at once; each then reads
//! its own rows. The gate is what stops either from taking the whole pool.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Semaphore;

use crate::state::UiRpcDbPool;

/// Seconds a subscription poll or snapshot build waited for a [`PollGate`]
/// permit.
pub(crate) const POLL_GATE_WAIT: &str = "moatless_ui_rpc_poll_gate_wait_seconds";

/// Bounds how many subscription polls and snapshot builds run at once on this
/// replica. Work over the bound waits for a permit; none is refused.
///
/// **Never nest [`Self::run`]:** gated work that awaits another gated call
/// holds one permit while waiting for a second, and deadlocks once every
/// permit is held that way.
#[derive(Clone, Debug)]
pub(crate) struct PollGate(Arc<Semaphore>);

impl PollGate {
    /// Half of `pool`'s connections, at least one. Gated work reads
    /// sequentially, so a permit stands for one connection and the other half
    /// stays free for workers and request handlers.
    pub(crate) fn for_pool(pool: &UiRpcDbPool) -> Self {
        let max = pool.options().get_max_connections() as usize;
        Self::new((max / 2).max(1))
    }

    pub(crate) fn new(permits: usize) -> Self {
        metrics::describe_histogram!(
            POLL_GATE_WAIT,
            "T3 UI-RPC seconds a subscription poll or snapshot waited for a database permit"
        );
        Self(Arc::new(Semaphore::new(permits)))
    }

    /// Run `work` once a permit is free, holding it until `work` finishes.
    pub(crate) async fn run<F: Future>(&self, work: F) -> F::Output {
        let waiting = Instant::now();
        // Never closed, so `acquire` only fails on a semaphore nobody owns.
        let _permit = self.0.acquire().await.ok();
        metrics::histogram!(POLL_GATE_WAIT).record(waiting.elapsed().as_secs_f64());
        work.await
    }
}

/// A uniformly random delay up to `interval`, for a poll loop's first sleep.
/// Subscriptions opened by one reconnect wave otherwise poll in step forever.
pub(crate) fn first_tick(interval: Duration) -> Duration {
    interval.mul_f64(rand::random::<f64>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn work_beyond_the_permits_waits_rather_than_running() {
        let gate = PollGate::new(2);
        let (release, released) = tokio::sync::watch::channel(false);
        let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut held = Vec::new();
        for _ in 0..5 {
            let gate = gate.clone();
            let running = running.clone();
            let mut released = released.clone();
            held.push(tokio::spawn(async move {
                gate.run(async {
                    running.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let _ = released.wait_for(|done| *done).await;
                })
                .await;
            }));
        }
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }

        assert_eq!(running.load(std::sync::atomic::Ordering::SeqCst), 2);
        let _ = release.send(true);
        for task in held {
            task.await.unwrap_or_default();
        }
        assert_eq!(running.load(std::sync::atomic::Ordering::SeqCst), 5);
    }

    #[test]
    fn a_first_tick_never_waits_longer_than_an_interval() {
        let interval = Duration::from_millis(250);
        for _ in 0..1000 {
            assert!(first_tick(interval) <= interval);
        }
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "poll_gate_db_tests.rs"]
mod db_tests;
