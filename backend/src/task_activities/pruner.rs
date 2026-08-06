//! The background worker that keeps `task_conversation_changes` bounded.
//! Registered with the root [`Supervisor`](crate::supervisor::Supervisor) in
//! `main` as `conversation-change-pruner`, and unconditional rather than gated
//! on the T3 compatibility module for the reason its sibling is: the records
//! are written by Moatless mutations whether or not anything is subscribed.

use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::dao::prune_changes;
use super::models::RETAINED_CHANGES;
use moatless_core::db::DbPool;

/// How often the pruner deletes what has fallen out of the retention window.
/// More often than the shell's, because this feed records a row per Message
/// written rather than per Task whose listing row moved.
const PRUNE_INTERVAL: Duration = Duration::from_mins(2);

/// Run [`prune_changes`] on a timer until cancelled. A failed prune is a
/// warning and nothing more: it deletes rows no reader is allowed to ask for —
/// [`resume`](super::models::resume) has already refused them — so falling
/// behind costs disk and the next tick covers the same ground.
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
                        "Pruned conversation changes past the resume window"
                    ),
                    Err(error) => warn!(%error, "Failed to prune conversation changes"),
                },
            }
        }
        info!("Conversation change pruner stopped");
    })
}
