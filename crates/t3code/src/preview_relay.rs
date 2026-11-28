//! The hop that carries a tab the environment opened to the other replicas.
//!
//! [`PreviewTabs`](crate::preview_tabs::PreviewTabs) fans out in this process
//! only, so a tab opened by a load-balanced HTTP call never reaches a viewer
//! socketed to a different replica. [`PreviewRelay::announce`] records the open
//! on the durable feed and [`PreviewRelayWorker`] applies everyone else's.

use std::time::Duration;

use moatless_core::change_feed::preview as feed;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval, sleep};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::preview::{self, PreviewState};
use crate::state::UiRpcDbPool;

/// How often a replica asks what the others have opened.
///
/// Fast, because what waits on it is a person who just pressed Run and is
/// looking at an empty browser panel.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How often a replica reclaims the opens past the retention window. Every
/// replica prunes; the `DELETE` is idempotent and takes no lease.
const PRUNE_INTERVAL: Duration = Duration::from_mins(10);

/// How long [`head`] waits after a failed read. Not [`POLL_INTERVAL`]: a read
/// that fails slower than its own interval leaves an `interval` tick already
/// due, so the retry would carry no pause at all — which is what a connect
/// timeout or a saturated pool produces.
const HEAD_RETRY_BACKOFF: Duration = Duration::from_secs(1);

/// Writes this replica's environment-initiated opens to the feed the others
/// read.
#[derive(Clone)]
pub struct PreviewRelay {
    /// `None` keeps every tab in this process, which is what a unit test with
    /// no database wants.
    pool: Option<UiRpcDbPool>,
}

impl PreviewRelay {
    pub(crate) fn new(pool: UiRpcDbPool) -> Self {
        Self { pool: Some(pool) }
    }

    /// A relay that announces nothing, for a test with no database.
    #[cfg(test)]
    pub(crate) fn disabled() -> Self {
        Self { pool: None }
    }

    /// Announce that `origin` opened `tab_id` on `task_id`, so the replicas
    /// holding the other viewers' sockets open it too.
    ///
    /// **Failure policy:** best effort. The tab is already stored and published
    /// on the origin, so a failure costs the other replicas this one tab; the
    /// preview URL still reaches it.
    pub(crate) async fn announce(&self, origin: &str, task_id: &str, tab_id: &str, url: &str) {
        let Some(pool) = &self.pool else {
            return;
        };
        match feed::record_open(pool, origin, task_id, tab_id, url).await {
            Ok(position) => {
                debug!(position, task_id, tab_id, "Announced a preview tab");
            }
            Err(error) => {
                warn!(
                    %error, task_id, tab_id,
                    "A preview tab was not announced to the other replicas"
                );
            }
        }
    }
}

/// The reading half: applies the other replicas' opens into this one's store.
///
/// Separate from [`PreviewRelay`] because it runs once per process under the
/// root supervisor, while the writing half is cloned into every handler that
/// opens a tab.
pub struct PreviewRelayWorker {
    preview: PreviewState,
}

impl PreviewRelayWorker {
    pub(crate) fn new(preview: PreviewState) -> Self {
        Self { preview }
    }

    /// Poll the feed until `cancel`, opening each tab another replica opened.
    ///
    /// The cursor starts at the feed's head, deliberately: this replica holds
    /// no subscriber older than itself, and replaying the window would raise
    /// tabs for Scripts that have since stopped.
    pub fn spawn(self, cancel: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            let Some(pool) = self.preview.relay.pool.clone() else {
                info!("The preview tab relay has no database handle; tabs stay in this process");
                return;
            };
            let origin = self.preview.tabs.epoch().to_string();
            let Some(mut cursor) = head(&pool, &cancel).await else {
                return;
            };

            let mut poller = interval(POLL_INTERVAL);
            poller.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut pruner = interval(PRUNE_INTERVAL);
            pruner.set_missed_tick_behavior(MissedTickBehavior::Skip);
            info!(origin, cursor, "The preview tab relay started");
            loop {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    _ = poller.tick() => {
                        match feed::opens_after(&pool, cursor, feed::CHANGES_PER_POLL).await {
                            Ok(opens) => {
                                for open in opens {
                                    // Its own records are already applied: this
                                    // replica opened the tab before writing
                                    // them, and re-applying would announce the
                                    // same tab to its subscribers twice.
                                    if open.origin != origin {
                                        preview::apply_relayed_open(
                                            &self.preview, &open.task_id, &open.tab_id, &open.url,
                                        );
                                    }
                                    cursor = open.position;
                                }
                            }
                            // Transient: the cursor has not moved, so the next
                            // poll covers the same ground.
                            Err(error) => warn!(%error, "Failed to read the preview tab feed"),
                        }
                    }
                    _ = pruner.tick() => {
                        if let Err(error) = feed::prune_opens(&pool).await {
                            warn!(%error, "Failed to prune the preview tab feed");
                        }
                    }
                }
            }
            info!("The preview tab relay stopped");
        })
    }
}

/// The position to start reading after, retrying until the database answers.
///
/// `None` once `cancel` fires. Retried rather than defaulted: starting at 0
/// would replay the whole retained window into every connected client.
async fn head(pool: &UiRpcDbPool, cancel: &CancellationToken) -> Option<i64> {
    loop {
        tokio::select! {
            () = cancel.cancelled() => return None,
            result = feed::current_position(pool) => match result {
                Ok(position) => return Some(position),
                Err(error) => warn!(
                    %error,
                    "The preview tab relay could not read the feed head; retrying"
                ),
            },
        }
        tokio::select! {
            () = cancel.cancelled() => return None,
            () = sleep(HEAD_RETRY_BACKOFF) => {}
        }
    }
}
