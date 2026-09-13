//! Fans the task-event change feed out to this pod's SSE clients.
//!
//! One cursor per pod, held in memory and started at the head, because every
//! pod must see every event — a shared cursor would hand each event to one pod
//! and starve the clients connected to the others.

use moatless_core::change_feed::events as feed;
use sqlx::PgPool;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// How long the loop waits between polls when the last one came back empty.
///
/// The floor on how stale a browser's live tail can be, so it is short; a
/// non-empty poll skips it entirely and reads again at once.
const IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// How many events one poll takes. A full batch means more is waiting, so the
/// loop reads again without idling.
const BATCH: i64 = 256;

/// How long the loop waits after a failed poll.
const ERROR_BACKOFF: std::time::Duration = std::time::Duration::from_secs(1);

/// Pushes newly recorded events onto this pod's SSE broadcast bus.
pub struct EventFeed {
    pool: PgPool,
    tx: broadcast::Sender<String>,
    cancel: CancellationToken,
}

impl EventFeed {
    pub fn new(pool: PgPool, tx: broadcast::Sender<String>, cancel: CancellationToken) -> Self {
        Self { pool, tx, cancel }
    }

    /// Poll the feed until cancelled.
    ///
    /// Starts at the head rather than 0: SSE is the live tail, and history is
    /// served from `task_events` by its own endpoint.
    pub async fn run(self) {
        let mut cursor = match self.start_position().await {
            Some(position) => position,
            None => return,
        };
        info!(cursor, "SSE event feed started");

        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            let batch = match feed::changes_after(&self.pool, cursor, BATCH).await {
                Ok(batch) => batch,
                Err(e) => {
                    warn!(error = %e, cursor, "SSE event feed poll failed, backing off");
                    if self.sleep(ERROR_BACKOFF).await {
                        break;
                    }
                    continue;
                }
            };

            let full = batch.len() as i64 == BATCH;
            for change in batch {
                cursor = change.position;
                // A send fails only when no SSE client is connected to this
                // pod, which is the common case and not an error.
                let _ = self.tx.send(change.event);
            }

            if !full && self.sleep(IDLE_POLL).await {
                break;
            }
        }

        info!("SSE event feed stopped");
    }

    /// Where to start reading; `None` when the loop should not start at all.
    async fn start_position(&self) -> Option<i64> {
        loop {
            match feed::current_position(&self.pool).await {
                Ok(position) => return Some(position),
                Err(e) => {
                    // Retry rather than give up: a pod that starts before the
                    // database is reachable would otherwise serve every client
                    // it ever accepts an empty stream.
                    warn!(error = %e, "SSE event feed cannot read the head yet, retrying");
                    if self.sleep(ERROR_BACKOFF).await {
                        return None;
                    }
                }
            }
        }
    }

    /// Sleep, or return `true` if cancelled first.
    async fn sleep(&self, duration: std::time::Duration) -> bool {
        tokio::select! {
            () = tokio::time::sleep(duration) => false,
            () = self.cancel.cancelled() => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_cancelled_feed_stops_sleeping_immediately() {
        let cancel = CancellationToken::new();
        let (tx, _rx) = broadcast::channel(4);
        let feed = EventFeed::new(
            PgPool::connect_lazy("postgres://invalid/invalid").expect("lazy pool"),
            tx,
            cancel.clone(),
        );
        cancel.cancel();
        assert!(feed.sleep(std::time::Duration::from_secs(30)).await);
    }
}
