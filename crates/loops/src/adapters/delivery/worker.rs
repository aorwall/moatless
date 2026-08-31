//! The supervised owner of admitted Adapter deliveries.
//!
//! One worker runs per backend process. It claims due records for the kinds
//! this build knows how to execute, runs each handler under a deadline and in
//! its own task, and drives every record to a terminal state.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures::FutureExt;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use moatless_adapter_model::delivery::AdapterDeliveryHandlers;
use super::metrics::{record_backlog, record_execution};
use moatless_adapter_model::delivery::{AdapterDelivery, AdapterDeliveryFailure};
use super::store::AdapterDeliveryStore;

/// Run the Adapter delivery worker until the root token is cancelled.
///
/// The join handle is returned for the root supervisor to own — see the
/// criticality declared at its registration in `main` — and is `#[must_use]`
/// because this task owns already-acknowledged sender work: detaching it
/// would silently strand records the backend has already promised to run.
///
/// The handle is the raw [`tokio::task::JoinHandle`] rather than a wrapper
/// type, because `Supervisor::supervise` takes exactly that and is this
/// function's only caller; there is no second consumer to poll or to join it
/// on its own deadline.
#[must_use = "this handle owns admitted Adapter deliveries; dropping it detaches the worker"]
pub fn spawn_adapter_delivery_worker(
    store: Arc<AdapterDeliveryStore>,
    handlers: Arc<AdapterDeliveryHandlers>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let owner = worker_owner_id();
    info!(
        owner = %owner,
        kinds = handlers.kinds().len(),
        "Adapter delivery worker spawned"
    );
    tokio::spawn(run(store, handlers, owner, cancel))
}

/// The claim identity written onto records this process holds.
///
/// Deliberately the pod/container hostname rather than a per-process random id:
/// a process that was killed mid-execution comes back under the same name, and
/// `release_claims_by_owner` then recovers its interrupted work at startup
/// instead of waiting out the claim lease. The lease still covers the owner
/// that never returns.
///
/// The fallback is unique per process rather than a shared constant, which
/// matters because a shared one would be actively harmful: every replica would
/// claim under the same name, and a starting replica would hand back records a
/// live replica is still executing. Uniqueness costs only immediate restart
/// recovery — a process that cannot see `HOSTNAME` waits out the lease instead
/// — which is what the lease is for. Delaying work that is already dead beats
/// disturbing work that is still running.
fn worker_owner_id() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| format!("unknown-host-{}", uuid::Uuid::new_v4()))
}

async fn run(
    store: Arc<AdapterDeliveryStore>,
    handlers: Arc<AdapterDeliveryHandlers>,
    owner: String,
    cancel: CancellationToken,
) {
    // Restart recovery: anything this owner was holding when it died is due
    // again immediately.
    match store.release_claims_by_owner(&owner).await {
        Ok(0) => {}
        Ok(recovered) => info!(
            owner = %owner,
            recovered,
            "Recovered Adapter deliveries claimed by a previous run of this worker"
        ),
        Err(e) => warn!(error = %e, owner = %owner, "Failed to recover this worker's claims"),
    }

    let policy = *store.policy();
    let mut poll = tokio::time::interval(policy.poll_interval);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut maintenance = tokio::time::interval(policy.maintenance_interval);
    maintenance.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = maintenance.tick() => run_maintenance(&store).await,
            _ = poll.tick() => drain_due(&store, &handlers, &owner).await,
        }
    }

    // Shutdown drains by *not* claiming another batch: the batch in flight is
    // awaited inside `drain_due` before the loop can observe cancellation.
    //
    // That await is not a promise the batch finishes. The root supervisor only
    // bounded-joins for `SHUTDOWN_JOIN_DEADLINE` (20s) whereas one execution may
    // take `execution_timeout` (2min), so a deploy landing on a long batch exits
    // with handlers still running and names this task a straggler. Nothing is
    // lost when it does: those records stay `claimed` and are recovered by the
    // next run of this owner or by lease expiry. Shutdown delays admitted work,
    // it never loses it.
    info!("Adapter delivery worker stopped");
}

/// Claim one batch and run it to completion. Bounded by `batch_size`, so the
/// worker never holds more in-flight work than it claimed.
async fn drain_due(
    store: &Arc<AdapterDeliveryStore>,
    handlers: &Arc<AdapterDeliveryHandlers>,
    owner: &str,
) {
    let claimed = match store.claim_due(owner, &handlers.kinds()).await {
        Ok(claimed) => claimed,
        Err(e) => {
            warn!(error = %e, "Failed to claim Adapter deliveries");
            return;
        }
    };
    if claimed.is_empty() {
        return;
    }

    let mut running = JoinSet::new();
    for delivery in claimed {
        let store = Arc::clone(store);
        let handlers = Arc::clone(handlers);
        // One task per record: a handler that wedges or dies cannot stall the
        // records claimed beside it.
        running.spawn(async move { execute(&store, &handlers, delivery).await });
    }
    while let Some(joined) = running.join_next().await {
        if let Err(e) = joined {
            // `execute` already catches handler panics, so reaching here means
            // the runtime tore the task down. The record stays claimed and its
            // lease recovers it.
            warn!(error = %e, "An Adapter delivery execution task was lost");
        }
    }
}

/// Run one claimed record and drive it to its next state.
async fn execute(
    store: &AdapterDeliveryStore,
    handlers: &AdapterDeliveryHandlers,
    delivery: AdapterDelivery,
) {
    let Some(handler) = handlers.get(delivery.kind.as_str()) else {
        // Unreachable while claims are filtered by registered kind. If it ever
        // happens the record is retried rather than made terminal, so a replica
        // that knows the kind can pick it up. The attempt charged at claim time
        // is not refunded — the retry budget is spent either way, which is what
        // stops an unknown kind from being retried forever.
        warn!(kind = %delivery.kind, id = %delivery.id, "A claimed Adapter delivery has no registered handler");
        finish_failed(store, &delivery, false, "no handler registered for kind").await;
        return;
    };

    let timeout = store.policy().execution_timeout;
    let started = std::time::Instant::now();
    // `catch_unwind` is what makes a poison record a *recorded* failure rather
    // than a claim that quietly waits out its lease every time it is retried.
    let attempt = tokio::time::timeout(
        timeout,
        AssertUnwindSafe(handler.execute(&delivery)).catch_unwind(),
    )
    .await;

    match attempt {
        Ok(Ok(Ok(()))) => {
            if let Err(e) = store.mark_succeeded(&delivery.id).await {
                // The handler did its job; the record stays claimed, its lease
                // expires, and the work runs again. Handlers are required to be
                // idempotent precisely for this window.
                warn!(id = %delivery.id, error = %e, "Failed to mark an Adapter delivery succeeded");
            }
            record_execution(&delivery.adapter, &delivery.kind, "succeeded");
            info!(
                id = %delivery.id,
                adapter = %delivery.adapter,
                kind = %delivery.kind,
                event_id = %delivery.event_id,
                attempt = delivery.attempts,
                duration_ms = started.elapsed().as_millis() as u64,
                "Adapter delivery completed"
            );
        }
        Ok(Ok(Err(failure))) => {
            let terminal = failure.is_permanent() || delivery.is_last_attempt();
            finish_failed(store, &delivery, terminal, &describe(&failure)).await;
        }
        Ok(Err(panic)) => {
            let reason = format!("handler panicked: {}", panic_message(&panic));
            finish_failed(store, &delivery, delivery.is_last_attempt(), &reason).await;
        }
        Err(_elapsed) => {
            let reason = format!(
                "handler exceeded the {}s execution deadline",
                timeout.as_secs()
            );
            finish_failed(store, &delivery, delivery.is_last_attempt(), &reason).await;
        }
    }
}

/// Record a failed attempt, and say so loudly when it was the last one.
///
/// A terminal record logs at `error` and increments the `terminal` outcome, so
/// an abandoned webhook is never indistinguishable from one that never arrived.
async fn finish_failed(
    store: &AdapterDeliveryStore,
    delivery: &AdapterDelivery,
    terminal: bool,
    reason: &str,
) {
    if terminal {
        error!(
            id = %delivery.id,
            adapter = %delivery.adapter,
            kind = %delivery.kind,
            event_id = %delivery.event_id,
            attempts = delivery.attempts,
            reason,
            "Adapter delivery abandoned after terminal failure"
        );
        record_execution(&delivery.adapter, &delivery.kind, "terminal");
    } else {
        warn!(
            id = %delivery.id,
            adapter = %delivery.adapter,
            kind = %delivery.kind,
            event_id = %delivery.event_id,
            attempt = delivery.attempts,
            max_attempts = delivery.max_attempts,
            reason,
            "Adapter delivery attempt failed; retrying with backoff"
        );
        record_execution(&delivery.adapter, &delivery.kind, "retry");
    }

    if let Err(e) = store
        .mark_attempt_failed(&delivery.id, terminal, delivery.attempts, reason)
        .await
    {
        warn!(id = %delivery.id, error = %e, "Failed to record an Adapter delivery attempt failure");
    }
}

/// Flatten a failure and its source chain into the stored `last_error`. The
/// chain is what makes a terminal record diagnosable weeks later.
fn describe(failure: &AdapterDeliveryFailure) -> String {
    let mut rendered = failure.to_string();
    let mut source = std::error::Error::source(failure);
    while let Some(cause) = source {
        rendered.push_str(": ");
        rendered.push_str(&cause.to_string());
        source = cause.source();
    }
    rendered
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else {
        "unknown panic payload"
    }
}

/// Recover stale claims, publish the backlog gauges, and prune finished
/// records.
async fn run_maintenance(store: &AdapterDeliveryStore) {
    match store.release_expired_claims().await {
        Ok(0) => {}
        Ok(released) => warn!(
            released,
            "Recovered Adapter deliveries whose claim lease expired"
        ),
        Err(e) => warn!(error = %e, "Failed to recover expired Adapter delivery claims"),
    }

    match store.backlog().await {
        Ok(backlog) => record_backlog(&backlog),
        Err(e) => warn!(error = %e, "Failed to read the Adapter delivery backlog"),
    }

    match store.prune_finished().await {
        Ok(0) => {}
        Ok(pruned) => info!(pruned, "Pruned finished Adapter delivery records"),
        Err(e) => warn!(error = %e, "Failed to prune finished Adapter delivery records"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_renders_the_whole_source_chain() {
        #[derive(Debug, thiserror::Error)]
        #[error("outer")]
        struct Outer(#[source] Inner);

        #[derive(Debug, thiserror::Error)]
        #[error("inner")]
        struct Inner;

        let rendered = describe(&AdapterDeliveryFailure::retryable(Outer(Inner)));
        assert_eq!(rendered, "retryable Adapter delivery failure: outer: inner");
    }

    #[test]
    fn panic_messages_survive_both_payload_shapes() {
        let boxed: Box<dyn std::any::Any + Send> = Box::new("static boom");
        assert_eq!(panic_message(boxed.as_ref()), "static boom");
        let boxed: Box<dyn std::any::Any + Send> = Box::new("owned boom".to_string());
        assert_eq!(panic_message(boxed.as_ref()), "owned boom");
        let boxed: Box<dyn std::any::Any + Send> = Box::new(7u8);
        assert_eq!(panic_message(boxed.as_ref()), "unknown panic payload");
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test assertions; the lints only fire when the db-tests feature is enabled"
)]
mod db_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use sqlx::PgPool;

    use moatless_adapter_model::delivery::{
        AdapterDeliveryHandler, AdapterDeliveryRequest, Admission, AdmitError,
    };
    use super::super::models::AdapterDeliveryPolicy;
    use super::*;

    /// Timings collapsed so retry, lease expiry, and recovery all happen inside
    /// a test rather than over ten minutes.
    fn fast_policy() -> AdapterDeliveryPolicy {
        AdapterDeliveryPolicy {
            max_attempts: 3,
            base_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
            // Long enough that maintenance cannot reclaim a record that is
            // still executing; the lease-expiry case is exercised by ageing
            // `claimed_at` directly instead of by waiting.
            claim_lease: Duration::from_secs(30),
            execution_timeout: Duration::from_secs(5),
            batch_size: 8,
            poll_interval: Duration::from_millis(5),
            maintenance_interval: Duration::from_millis(5),
            backlog_capacity: 10_000,
            succeeded_retention: Duration::from_hours(24 * 7),
            failed_retention: Duration::from_hours(24 * 30),
        }
    }

    const KIND: &str = "test.delivery";
    const OTHER_KIND: &str = "test.other";

    /// A handler whose outcome the test dictates, counting every execution so
    /// "ran exactly once" is assertable.
    struct ScriptedHandler {
        runs: Arc<AtomicUsize>,
        outcome: Outcome,
    }

    #[derive(Clone, Copy)]
    enum Outcome {
        Succeed,
        Retryable,
        Permanent,
        Panic,
    }

    #[async_trait::async_trait]
    impl AdapterDeliveryHandler for ScriptedHandler {
        async fn execute(&self, _delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            match self.outcome {
                Outcome::Succeed => Ok(()),
                Outcome::Retryable => Err(AdapterDeliveryFailure::retryable(
                    std::io::Error::other("sender unavailable"),
                )),
                Outcome::Permanent => Err(AdapterDeliveryFailure::permanent(
                    std::io::Error::other("task was deleted"),
                )),
                Outcome::Panic => panic!("poison record"),
            }
        }
    }

    fn handler(
        kind: &'static str,
        outcome: Outcome,
    ) -> (AdapterDeliveryHandlers, Arc<AtomicUsize>) {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut handlers = AdapterDeliveryHandlers::new();
        handlers.register(
            kind,
            Arc::new(ScriptedHandler {
                runs: Arc::clone(&runs),
                outcome,
            }),
        );
        (handlers, runs)
    }

    fn request(kind: &'static str, event_id: &str) -> AdapterDeliveryRequest {
        AdapterDeliveryRequest {
            adapter: "test",
            kind,
            event_id: event_id.to_string(),
            payload: serde_json::json!({ "text": "hello" }),
        }
    }

    /// The claim identity a restart-recovery test pins its worker to.
    ///
    /// Fixed rather than derived, because `worker_owner_id` falls back to a
    /// fresh per-process id when `HOSTNAME` is unset. Deriving it would make
    /// restart recovery — which is owner-matched — depend on the environment
    /// the suite happens to run in.
    const TEST_OWNER: &str = "test-worker";

    /// Drive the worker until `done` holds or `budget` elapses, then cancel
    /// it — the same start/cancel path `main` uses.
    async fn run_worker_until(
        store: &Arc<AdapterDeliveryStore>,
        handlers: Arc<AdapterDeliveryHandlers>,
        budget: Duration,
        done: impl Fn() -> bool + Send,
    ) {
        let cancel = CancellationToken::new();
        let handle = spawn_adapter_delivery_worker(Arc::clone(store), handlers, cancel.clone());
        drive(handle, cancel, budget, done).await;
    }

    /// As [`run_worker_until`], but with the worker's claim identity pinned.
    ///
    /// Only for tests that seed a `claimed` row and expect the worker to
    /// recognise it as its own. `worker_owner_id` derives a fresh per-process
    /// id when `HOSTNAME` is unset, so deriving it separately for the seed and
    /// for the worker would make owner-matched recovery depend on the
    /// environment. `run` already takes the owner as a parameter; this passes
    /// it rather than letting the worker compute a second one.
    async fn run_worker_until_as(
        store: &Arc<AdapterDeliveryStore>,
        handlers: Arc<AdapterDeliveryHandlers>,
        owner: &str,
        budget: Duration,
        done: impl Fn() -> bool + Send,
    ) {
        let cancel = CancellationToken::new();
        let handle = tokio::spawn(super::run(
            Arc::clone(store),
            handlers,
            owner.to_string(),
            cancel.clone(),
        ));
        drive(handle, cancel, budget, done).await;
    }

    /// Wait for `done` or `budget`, then cancel and join.
    async fn drive(
        handle: tokio::task::JoinHandle<()>,
        cancel: CancellationToken,
        budget: Duration,
        done: impl Fn() -> bool + Send,
    ) {
        let deadline = tokio::time::Instant::now() + budget;
        while tokio::time::Instant::now() < deadline && !done() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cancel.cancel();
        handle.await.expect("worker exits cleanly on cancellation");
    }

    fn store(pool: PgPool) -> Arc<AdapterDeliveryStore> {
        Arc::new(AdapterDeliveryStore::with_policy(pool, fast_policy()))
    }

    /// Verification: duplicate webhook delivery. A sender that re-delivers the
    /// same event must not produce a second record.
    #[sqlx::test]
    async fn a_duplicate_delivery_admits_once_and_executes_once(pool: PgPool) {
        let store = store(pool);
        assert_eq!(
            store.admit(request(KIND, "evt-1")).await.unwrap(),
            Admission::Admitted
        );
        assert_eq!(
            store.admit(request(KIND, "evt-1")).await.unwrap(),
            Admission::Duplicate,
            "the sender's delivery id, not a generated one, is the idempotency key"
        );

        let (handlers, runs) = handler(KIND, Outcome::Succeed);
        run_worker_until(&store, Arc::new(handlers), Duration::from_secs(5), || {
            runs.load(Ordering::SeqCst) > 0
        })
        .await;

        assert_eq!(runs.load(Ordering::SeqCst), 1, "work ran exactly once");
        let (status, attempts, _) = store.peek("test", KIND, "evt-1").await.unwrap().unwrap();
        assert_eq!(status, "succeeded");
        assert_eq!(attempts, 1);

        // A late re-delivery still deduplicates against the succeeded record.
        assert_eq!(
            store.admit(request(KIND, "evt-1")).await.unwrap(),
            Admission::Duplicate
        );
    }

    /// Verification: crash after HTTP acknowledgment. The admitting process
    /// never runs a worker at all — the work is picked up by a different one,
    /// which is exactly what a mid-request kill looks like from the queue's
    /// side.
    #[sqlx::test]
    async fn work_admitted_before_a_crash_is_executed_by_the_next_process(pool: PgPool) {
        let admitting = store(pool.clone());
        admitting.admit(request(KIND, "evt-crash")).await.unwrap();
        drop(admitting); // the process that acknowledged the sender is gone

        let recovering = store(pool);
        let (handlers, runs) = handler(KIND, Outcome::Succeed);
        run_worker_until(
            &recovering,
            Arc::new(handlers),
            Duration::from_secs(5),
            || runs.load(Ordering::SeqCst) > 0,
        )
        .await;

        assert_eq!(runs.load(Ordering::SeqCst), 1);
        let (status, ..) = recovering
            .peek("test", KIND, "evt-crash")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, "succeeded");
    }

    /// Verification: process-restart recovery. A record left `claimed` by a
    /// killed worker is reclaimed — immediately when the same owner returns,
    /// and by lease expiry when it does not.
    #[sqlx::test]
    async fn claims_left_by_a_killed_worker_are_recovered(pool: PgPool) {
        let store = store(pool.clone());
        store.admit(request(KIND, "evt-restart")).await.unwrap();
        store.admit(request(KIND, "evt-orphan")).await.unwrap();

        // Simulate the kill: both records were claimed and never finished, one
        // by the owner that is about to restart and one by a replica that is
        // never coming back. The first is recovered by owner match, the second
        // only by lease expiry, so the two paths are exercised separately.
        sqlx::query("UPDATE adapter_delivery SET status = 'claimed', attempts = 1, claimed_at = NOW(), claimed_by = $1 WHERE event_id = 'evt-restart'")
            .bind(TEST_OWNER)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE adapter_delivery SET status = 'claimed', attempts = 1, claimed_at = NOW() - INTERVAL '1 hour', claimed_by = 'dead-replica' WHERE event_id = 'evt-orphan'")
            .execute(&pool)
            .await
            .unwrap();

        // The worker restarts under the owner that seeded `evt-restart`, so the
        // two recovery paths are pinned rather than left to the environment:
        // `evt-restart` comes back by owner match, `evt-orphan` by lease expiry.
        let (handlers, runs) = handler(KIND, Outcome::Succeed);
        run_worker_until_as(
            &store,
            Arc::new(handlers),
            TEST_OWNER,
            Duration::from_secs(5),
            || runs.load(Ordering::SeqCst) >= 2,
        )
        .await;

        for event_id in ["evt-restart", "evt-orphan"] {
            let (status, ..) = store.peek("test", KIND, event_id).await.unwrap().unwrap();
            assert_eq!(
                status, "succeeded",
                "{event_id} was recovered and completed"
            );
        }
    }

    /// Verification: poison-item isolation. A record that always fails burns
    /// its budget, goes terminal with its cause retained, and does not stop the
    /// records claimed beside it.
    #[sqlx::test]
    async fn a_poison_record_goes_terminal_without_blocking_healthy_work(pool: PgPool) {
        let store = store(pool);
        store.admit(request(KIND, "evt-poison")).await.unwrap();
        store
            .admit(request(OTHER_KIND, "evt-healthy"))
            .await
            .unwrap();

        let poison_runs = Arc::new(AtomicUsize::new(0));
        let healthy_runs = Arc::new(AtomicUsize::new(0));
        let mut handlers = AdapterDeliveryHandlers::new();
        handlers.register(
            KIND,
            Arc::new(ScriptedHandler {
                runs: Arc::clone(&poison_runs),
                outcome: Outcome::Retryable,
            }),
        );
        handlers.register(
            OTHER_KIND,
            Arc::new(ScriptedHandler {
                runs: Arc::clone(&healthy_runs),
                outcome: Outcome::Succeed,
            }),
        );

        run_worker_until(&store, Arc::new(handlers), Duration::from_secs(5), || {
            poison_runs.load(Ordering::SeqCst) >= 3 && healthy_runs.load(Ordering::SeqCst) >= 1
        })
        .await;

        let (status, attempts, last_error) = store
            .peek("test", KIND, "evt-poison")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, "failed", "terminal after the budget");
        assert_eq!(attempts, 3, "attempts are capped by max_attempts");
        assert!(
            poison_runs.load(Ordering::SeqCst) == 3,
            "a terminal record is not retried again"
        );
        let last_error = last_error.expect("a terminal record says why it was abandoned");
        assert!(
            last_error.contains("sender unavailable"),
            "the handler's cause is retained: {last_error}"
        );

        let (status, ..) = store
            .peek("test", OTHER_KIND, "evt-healthy")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            status, "succeeded",
            "the poison record did not block healthy work"
        );
    }

    /// A panicking handler is isolated: the record records the panic and the
    /// worker keeps running.
    #[sqlx::test]
    async fn a_panicking_handler_is_isolated_and_recorded(pool: PgPool) {
        let store = store(pool);
        store.admit(request(KIND, "evt-panic")).await.unwrap();

        let (handlers, runs) = handler(KIND, Outcome::Panic);
        run_worker_until(&store, Arc::new(handlers), Duration::from_secs(5), || {
            runs.load(Ordering::SeqCst) >= 3
        })
        .await;

        let (status, attempts, last_error) = store
            .peek("test", KIND, "evt-panic")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(attempts, 3);
        assert!(
            last_error.unwrap_or_default().contains("poison record"),
            "the panic payload is retained on the record"
        );
    }

    /// A permanent failure skips the remaining budget entirely.
    #[sqlx::test]
    async fn a_permanent_failure_is_terminal_on_the_first_attempt(pool: PgPool) {
        let store = store(pool);
        store.admit(request(KIND, "evt-permanent")).await.unwrap();

        let (handlers, runs) = handler(KIND, Outcome::Permanent);
        run_worker_until(&store, Arc::new(handlers), Duration::from_secs(5), || {
            runs.load(Ordering::SeqCst) >= 1
        })
        .await;

        let (status, attempts, last_error) = store
            .peek("test", KIND, "evt-permanent")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(attempts, 1, "no attempt budget is spent on retrying");
        assert!(last_error.unwrap_or_default().contains("task was deleted"));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    /// Admission is bounded: past the configured backlog the seam refuses the
    /// work rather than accepting what it cannot promise to run.
    #[sqlx::test]
    async fn admission_refuses_work_past_the_backlog_capacity(pool: PgPool) {
        let policy = AdapterDeliveryPolicy {
            backlog_capacity: 2,
            ..fast_policy()
        };
        let store = AdapterDeliveryStore::with_policy(pool, policy);

        store.admit(request(KIND, "evt-1")).await.unwrap();
        store.admit(request(KIND, "evt-2")).await.unwrap();
        let err = store
            .admit(request(KIND, "evt-3"))
            .await
            .expect_err("the third admission is over capacity");
        assert!(matches!(err, AdmitError::AtCapacity { limit: 2 }));

        // A duplicate of already-admitted work is still recognised at capacity,
        // so a sender retry does not turn into a rejection storm.
        assert_eq!(
            store.admit(request(KIND, "evt-1")).await.unwrap(),
            Admission::Duplicate
        );
    }

    /// Under concurrency the capacity check is a stable ceiling, not an exact
    /// cutoff, and this pins both halves of that claim: a burst overshoots by
    /// at most the number of admissions that can be in flight, and once the
    /// committed backlog is at capacity nothing further gets in. Ratcheting —
    /// the ceiling drifting upward burst after burst — is the failure that
    /// would make ingress unbounded, and is what this rules out.
    #[sqlx::test]
    async fn concurrent_admission_overshoots_by_a_bounded_amount_and_never_ratchets(pool: PgPool) {
        const CAPACITY: i64 = 4;
        const BURST: usize = 24;

        let in_flight = i64::from(pool.options().get_max_connections());
        let store = Arc::new(AdapterDeliveryStore::with_policy(
            pool.clone(),
            AdapterDeliveryPolicy {
                backlog_capacity: CAPACITY,
                ..fast_policy()
            },
        ));

        let mut burst = JoinSet::new();
        for i in 0..BURST {
            let store = Arc::clone(&store);
            burst.spawn(async move {
                store
                    .admit(request(KIND, &format!("evt-burst-{i}")))
                    .await
                    .is_ok()
            });
        }
        let mut admitted: i64 = 0;
        while let Some(joined) = burst.join_next().await {
            if joined.expect("no admission task panics") {
                admitted += 1;
            }
        }

        assert!(
            admitted >= CAPACITY,
            "the guard must not under-admit: {admitted} < {CAPACITY}"
        );
        // Equivalent to the documented `capacity + C - 1` ceiling, written as a
        // strict `<` because clippy rejects the `<= x - 1` form.
        assert!(
            admitted < CAPACITY + in_flight,
            "overshoot is bounded by the admissions that can run at once: \
             {admitted} > {CAPACITY} + {in_flight} - 1"
        );
        assert_eq!(unfinished(&pool).await, admitted);

        // Stability: with the committed backlog already at the ceiling, every
        // further admission is refused, so the ceiling cannot drift upward.
        for i in 0..4 {
            let err = store
                .admit(request(KIND, &format!("evt-after-{i}")))
                .await
                .expect_err("nothing is admitted once the backlog is at capacity");
            assert!(matches!(err, AdmitError::AtCapacity { .. }));
        }
        assert_eq!(
            unfinished(&pool).await,
            admitted,
            "the ceiling did not ratchet upward"
        );
    }

    /// Unfinished (`pending` + `claimed`) records — the set the capacity
    /// predicate counts.
    async fn unfinished(pool: &PgPool) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM adapter_delivery WHERE status IN ('pending', 'claimed')",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// A record whose kind no handler in this build knows is left alone rather
    /// than charged attempts it could never pass — the rolling-deploy case.
    #[sqlx::test]
    async fn work_for_an_unknown_kind_is_left_for_a_replica_that_knows_it(pool: PgPool) {
        let store = store(pool);
        store
            .admit(request(OTHER_KIND, "evt-future"))
            .await
            .unwrap();

        let (handlers, runs) = handler(KIND, Outcome::Succeed);
        // Nothing can complete, so this just gives the worker time to prove
        // it leaves the record alone.
        run_worker_until(
            &store,
            Arc::new(handlers),
            Duration::from_millis(200),
            || false,
        )
        .await;

        assert_eq!(runs.load(Ordering::SeqCst), 0);
        let (status, attempts, _) = store
            .peek("test", OTHER_KIND, "evt-future")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(attempts, 0, "an unclaimable record spends no attempts");
    }
}
