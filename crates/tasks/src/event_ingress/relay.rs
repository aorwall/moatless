//! The ingress outbox relay: re-publication of admitted payloads.
//!
//! On the healthy path the consumer commits `task_messages` and deletes the
//! record before the relay ever sees it. The relay exists for the other path —
//! a failed publish, a Redis outage, a dead consumer — and re-publishes
//! unconfirmed records until one lands.
//!
//! **Only confirmed delivery deletes a record**, via
//! [`IngressOutbox::delete_delivered`] called by the event-handler consumer
//! after it commits; the relay itself never deletes. Re-publication re-stamps
//! the envelope's timestamp, so a recovered message carries the recovery time,
//! not its admission time.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use moatless_core::events::EventPublisher;

use super::outbox_dao::{IngressOutbox, RETRY_CAP_SECS};

/// How often the relay looks for records whose delivery is unconfirmed.
const POLL_INTERVAL: Duration = Duration::from_secs(15);

/// Records re-published per pass. Bounds one pass's work; anything left over is
/// still due on the next tick.
const BATCH_SIZE: i64 = 100;

/// What one relay pass did. Returned so tests can drive a single pass instead
/// of racing the loop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelayPass {
    /// Records successfully re-published to the consumer path.
    pub republished: usize,
    /// Records whose re-publication failed; they stay for the next pass.
    pub failed: usize,
}

/// Re-publishes ingress-outbox records whose delivery is unconfirmed.
pub struct IngressRelay {
    outbox: Arc<dyn IngressOutbox>,
    publisher: Arc<dyn EventPublisher>,
    poll_interval: Duration,
}

impl IngressRelay {
    pub(crate) fn new(outbox: Arc<dyn IngressOutbox>, publisher: Arc<dyn EventPublisher>) -> Self {
        Self {
            outbox,
            publisher,
            poll_interval: POLL_INTERVAL,
        }
    }

    /// One pass: claim the due records and re-publish each.
    ///
    /// A claim reschedules its records in the same statement, so a pass that
    /// panics or is cancelled mid-flight loses nothing — the records simply
    /// come due again later.
    pub async fn relay_once(&self) -> RelayPass {
        let due = match self.outbox.claim_due(Utc::now(), BATCH_SIZE).await {
            Ok(due) => due,
            Err(e) => {
                warn!(error = %e, "ingress relay could not claim due records");
                metrics::counter!("event_ingress_relay_failures_total", "stage" => "claim")
                    .increment(1);
                return RelayPass::default();
            }
        };

        let mut pass = RelayPass::default();
        for record in due {
            let payload = record.payload;
            // `false`: the consumer rebuilds `task_messages` from this payload,
            // so it must go out whole even though a durable copy exists.
            let published = self
                .publisher
                .publish_raw_event_with_id(
                    &payload.event_id,
                    &payload.task_id,
                    &payload.event_type,
                    &payload.sub_type,
                    None,
                    payload.turn_number,
                    payload.payload,
                    false,
                )
                .await;

            match published {
                Ok(()) => {
                    pass.republished += 1;
                    info!(
                        task_id = %payload.task_id,
                        event_id = %payload.event_id,
                        attempts = record.attempts,
                        admitted_at = %record.admitted_at,
                        "republished an admitted payload whose delivery was unconfirmed"
                    );
                    metrics::counter!("event_ingress_relay_republished_total").increment(1);
                }
                Err(e) => {
                    pass.failed += 1;
                    warn!(
                        task_id = %payload.task_id,
                        event_id = %payload.event_id,
                        attempts = record.attempts,
                        error = %e,
                        "ingress relay failed to republish an admitted payload"
                    );
                    metrics::counter!("event_ingress_relay_failures_total", "stage" => "publish")
                        .increment(1);
                }
            }
        }

        self.report_pending().await;
        pass
    }

    /// Publish the outstanding-record gauges. A rising `pending` means payloads
    /// are being admitted faster than they are committed; a non-zero `stalled`
    /// means some payload has outlived every retry the relay has made for it.
    async fn report_pending(&self) {
        let stale_before = Utc::now() - chrono::Duration::seconds(i64::from(RETRY_CAP_SECS));
        match self.outbox.pending_counts(stale_before).await {
            Ok(pending) => {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "gauge values; an outbox depth beyond f64 precision is not reachable"
                )]
                {
                    metrics::gauge!("event_ingress_outbox_pending").set(pending.total as f64);
                    metrics::gauge!("event_ingress_outbox_stalled").set(pending.stale as f64);
                }
                if pending.stale > 0 {
                    warn!(
                        stalled = pending.stale,
                        pending = pending.total,
                        "ingress outbox holds payloads no consumer has committed"
                    );
                }
            }
            Err(e) => warn!(error = %e, "ingress relay could not read outbox depth"),
        }
    }

    /// Run until `cancel` fires, handing back the owned handle.
    ///
    /// The loop never returns early: a failing pass is logged and retried next
    /// tick, so termination means cancellation (or a panic, surfaced through
    /// the handle's join result).
    pub fn spawn(self, cancel: CancellationToken) -> IngressRelayHandle {
        let join = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(self.poll_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    _ = ticker.tick() => { self.relay_once().await; }
                }
            }
            info!("Ingress outbox relay stopped");
        });
        IngressRelayHandle { join }
    }
}

/// Owned handle to the spawned [`IngressRelay`], `#[must_use]` since dropping
/// it detaches critical work.
///
/// The backend answers gRPC success for an `agent.message` on the outbox record
/// alone; without a running relay, undelivered content never reaches
/// `task_messages`. Cancel via the token passed to [`IngressRelay::spawn`], or
/// join with [`Self::shutdown`].
#[must_use = "the relay handle owns critical work; dropping it detaches the task"]
pub struct IngressRelayHandle {
    join: JoinHandle<()>,
}

impl IngressRelayHandle {
    /// Whether the task has terminated — the readiness/liveness signal for a
    /// supervisor that polls rather than joins.
    pub fn is_finished(&self) -> bool {
        self.join.is_finished()
    }

    /// Surrender the raw join handle to a root supervisor.
    pub fn into_join_handle(self) -> JoinHandle<()> {
        self.join
    }

    /// Wait up to `deadline` for the (already-cancelled) relay to finish,
    /// reporting a straggler rather than blocking shutdown indefinitely.
    pub async fn shutdown(self, deadline: Duration) {
        match tokio::time::timeout(deadline, self.join).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!(error = %e, "ingress outbox relay terminated abnormally"),
            Err(_elapsed) => warn!(
                deadline_ms = deadline.as_millis() as u64,
                "ingress outbox relay did not stop within the shutdown deadline"
            ),
        }
    }
}
