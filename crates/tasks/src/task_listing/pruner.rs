//! The background worker that keeps `task_listing_changes` bounded.
//!
//! Registered with the root `Supervisor` in the backend binary's `main` as
//! `listing-change-pruner`, and unconditional rather than gated on the T3
//! compatibility module: the records are written by Moatless mutations whether
//! or not anything is subscribed.

use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use moatless_core::change_feed::listing::RETAINED_CHANGES;
use moatless_core::change_feed::listing::prune_changes;
use moatless_core::db::DbPool;

/// How often the pruner deletes what has fallen out of the retention window.
const PRUNE_INTERVAL: Duration = Duration::from_mins(5);

/// Run [`prune_changes`] on a timer until cancelled.
///
/// A failed prune only warns: it deletes rows [`resume`](super::models::resume)
/// already refused, so falling behind costs disk and the next tick catches up.
pub fn spawn_change_pruner(pool: DbPool, cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(PRUNE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                _ = ticker.tick() => match prune_changes(&pool).await {
                    Ok(0) => {}
                    Ok(deleted) => debug!(
                        deleted,
                        retained = RETAINED_CHANGES,
                        "Pruned listing changes past the resume window"
                    ),
                    Err(error) => warn!(%error, "Failed to prune listing changes"),
                },
            }
        }
        info!("Listing change pruner stopped");
    })
}
