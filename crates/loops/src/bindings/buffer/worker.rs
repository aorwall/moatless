//! The flusher that turns a closed buffer window into one Task message.
//!
//! It claims the bindings whose window has closed, hands each binding's events
//! to [`RoutingService::flush_buffered`], and drops the rows once the turn is
//! accepted.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::dao::{BufferedBatch, MessageBufferStore};
use crate::bindings::runtime::RoutingService;
use crate::worker_identity::worker_owner_id;

/// How often the flusher looks for a closed window. The window itself is
/// measured in tens of seconds, so this only bounds the overshoot.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How often expired claims are recovered.
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(30);

/// Bindings claimed per poll, each flushed in its own task, so this is also the
/// flusher's concurrency bound.
const BATCH_BINDINGS: i64 = 16;

/// The host's handle on the inbound-buffer flusher.
///
/// A buffered delivery is only half-done until this runs, so a host that takes
/// it from `crate::compose::LoopGraph` must supervise it.
#[derive(Clone)]
pub struct MessageBufferWorker {
    store: Arc<MessageBufferStore>,
}

impl MessageBufferWorker {
    pub(crate) fn new(store: Arc<MessageBufferStore>) -> Self {
        Self { store }
    }

    /// Run the flusher until the root token is cancelled.
    ///
    /// Hands back the raw [`tokio::task::JoinHandle`] `Supervisor::supervise`,
    /// its only caller, takes.
    #[must_use = "this handle owns buffered inbound events; dropping it detaches the flusher"]
    pub fn spawn(
        &self,
        routing: Arc<RoutingService>,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let owner = worker_owner_id();
        info!(owner = %owner, "Inbound message buffer flusher spawned");
        tokio::spawn(run(Arc::clone(&self.store), routing, owner, cancel))
    }
}

async fn run(
    store: Arc<MessageBufferStore>,
    routing: Arc<RoutingService>,
    owner: String,
    cancel: CancellationToken,
) {
    match store.release_claims_by_owner(&owner).await {
        Ok(0) => {}
        Ok(recovered) => info!(
            owner = %owner,
            recovered,
            "Recovered buffered inbound events claimed by a previous run of this flusher"
        ),
        Err(e) => warn!(error = %e, owner = %owner, "Failed to recover this flusher's claims"),
    }

    let mut poll = tokio::time::interval(POLL_INTERVAL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut maintenance = tokio::time::interval(MAINTENANCE_INTERVAL);
    maintenance.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = maintenance.tick() => match store.release_expired_claims().await {
                Ok(0) => {}
                Ok(released) => warn!(released, "Recovered buffered inbound events whose claim lease expired"),
                Err(e) => warn!(error = %e, "Failed to recover expired inbound buffer claims"),
            },
            _ = poll.tick() => drain_due(&store, &routing, &owner).await,
        }
    }

    info!("Inbound message buffer flusher stopped");
}

async fn drain_due(store: &Arc<MessageBufferStore>, routing: &Arc<RoutingService>, owner: &str) {
    let batches = match store.claim_due(owner, BATCH_BINDINGS).await {
        Ok(batches) => batches,
        Err(e) => {
            warn!(error = %e, "Failed to claim buffered inbound events");
            return;
        }
    };

    let mut running = JoinSet::new();
    for batch in batches {
        let store = Arc::clone(store);
        let routing = Arc::clone(routing);
        // One task per binding: a flush waiting on a slow Adapter API call
        // cannot hold back the bindings claimed beside it. The caller still
        // waits for the slowest of them before its next tick.
        running.spawn(async move { flush(&store, &routing, batch).await });
    }
    while let Some(joined) = running.join_next().await {
        if let Err(e) = joined {
            // The rows stay claimed and their lease recovers them.
            warn!(error = %e, "An inbound buffer flush task was lost");
        }
    }
}

/// Deliver one binding's buffered events, then drop or reschedule its rows.
async fn flush(store: &MessageBufferStore, routing: &RoutingService, batch: BufferedBatch) {
    match routing.flush_buffered(&batch).await {
        Ok(()) => {
            if let Err(e) = store.delete(&batch.row_ids).await {
                // The turn is on the transcript and the rows are still claimed;
                // their lease returns them and the task sees the events twice.
                // Rare, and visibly duplicated beats silently dropped.
                warn!(binding_id = %batch.binding_id, error = %e, "Failed to drop flushed inbound buffer rows");
            }
        }
        Err(e) if batch.is_last_attempt() => {
            error!(
                binding_id = %batch.binding_id,
                adapter = %batch.adapter_label,
                events = batch.messages.len(),
                error = %e,
                "Abandoning buffered inbound events after the last flush attempt"
            );
            if let Err(e) = store.delete(&batch.row_ids).await {
                warn!(binding_id = %batch.binding_id, error = %e, "Failed to drop abandoned inbound buffer rows");
            }
        }
        Err(e) => {
            warn!(
                binding_id = %batch.binding_id,
                adapter = %batch.adapter_label,
                attempt = batch.attempts,
                error = %e,
                "Buffered inbound flush failed; retrying with backoff"
            );
            if let Err(e) = store.reschedule(&batch.row_ids).await {
                warn!(binding_id = %batch.binding_id, error = %e, "Failed to reschedule inbound buffer rows");
            }
        }
    }
}
