//! The hop that carries a session the environment started to the other
//! replicas.
//!
//! A Script run over HTTP lands on whichever replica the load balancer picked,
//! and [`TerminalHub`](crate::terminal::TerminalHub) reaches only the sockets
//! that replica holds. The durable feed closes that gap.
//!
//! Built on `preview_relay`'s pattern, which documents the poll, the prune and
//! the head-read the two halves below share.

use std::time::Duration;

use moatless_core::change_feed::terminal as feed;
use serde_json::Value;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval, sleep};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::state::UiRpcDbPool;
use crate::terminal::TerminalHub;

/// How often a replica asks what the others have started. Short: a terminal
/// drawer sits empty for this long after someone presses Run.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How often a replica reclaims the starts past the retention window.
const PRUNE_INTERVAL: Duration = Duration::from_mins(10);

/// How long [`head`] waits after a failed read. Deliberately not
/// [`POLL_INTERVAL`] — `preview_relay`'s constant of the same name says what a
/// pause shorter than the failure itself costs.
const HEAD_RETRY_BACKOFF: Duration = Duration::from_secs(1);

/// Writes this replica's environment-started sessions to the feed the others
/// read.
#[derive(Clone)]
pub struct TerminalRelay {
    /// `None` confines sessions to this process, for a test with no database.
    pool: Option<UiRpcDbPool>,
}

impl TerminalRelay {
    pub(crate) fn new(pool: UiRpcDbPool) -> Self {
        Self { pool: Some(pool) }
    }

    /// A relay that announces nothing, for a test with no database.
    #[cfg(test)]
    pub(crate) fn disabled() -> Self {
        Self { pool: None }
    }

    /// Announce that `origin` started `terminal_id` on `task_id`, so the
    /// replicas holding the other viewers' sockets list it too.
    ///
    /// **Failure policy:** best effort, and logged. What a lost announcement
    /// costs is one terminal missing from the other replicas' listings until a
    /// reconnect rebuilds them; the session itself runs on regardless.
    pub(crate) async fn announce(
        &self,
        origin: &str,
        task_id: &str,
        terminal_id: &str,
        summary: &Value,
    ) {
        let Some(pool) = &self.pool else {
            return;
        };
        match feed::record_start(pool, origin, task_id, terminal_id, summary).await {
            Ok(position) => {
                debug!(
                    position,
                    task_id, terminal_id, "Announced a terminal session"
                );
            }
            Err(error) => {
                warn!(
                    %error, task_id, terminal_id,
                    "A terminal session was not announced to the other replicas"
                );
            }
        }
    }
}

/// The reading half: applies the other replicas' starts into this one's hub.
///
/// A type of its own because the supervisor owns exactly one of these per
/// process, whereas [`TerminalRelay`] lives inside the hub.
pub struct TerminalRelayWorker {
    hub: TerminalHub,
}

impl TerminalRelayWorker {
    pub(crate) fn new(hub: TerminalHub) -> Self {
        Self { hub }
    }

    /// Poll the feed until `cancel`, listing each session another replica
    /// started.
    ///
    /// Reads from the head onwards rather than from 0. Nothing retained
    /// predates this process, so a replay would only raise drawers for Scripts
    /// that have already exited.
    pub fn spawn(self, cancel: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            let Some(pool) = self.hub.relay().pool.clone() else {
                info!(
                    "The terminal session relay has no database handle; sessions stay in this process"
                );
                return;
            };
            let origin = self.hub.epoch().to_string();
            let Some(mut cursor) = head(&pool, &cancel).await else {
                return;
            };

            let mut poller = interval(POLL_INTERVAL);
            poller.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut pruner = interval(PRUNE_INTERVAL);
            pruner.set_missed_tick_behavior(MissedTickBehavior::Skip);
            info!(origin, cursor, "The terminal session relay started");
            loop {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    _ = poller.tick() => {
                        match feed::starts_after(&pool, cursor, feed::CHANGES_PER_POLL).await {
                            Ok(starts) => {
                                for start in starts {
                                    // Skipped. The hub stored this session
                                    // before the record existed, so applying it
                                    // here would show the drawer twice.
                                    if start.origin != origin {
                                        self.hub.apply_relayed_start(
                                            &start.task_id, &start.terminal_id, start.summary,
                                        )
                                        .await;
                                    }
                                    cursor = start.position;
                                }
                            }
                            // Transient; `cursor` is unmoved and the next tick
                            // re-reads from it.
                            Err(error) => warn!(%error, "Failed to read the terminal session feed"),
                        }
                    }
                    _ = pruner.tick() => {
                        if let Err(error) = feed::prune_starts(&pool).await {
                            warn!(%error, "Failed to prune the terminal session feed");
                        }
                    }
                }
            }
            info!("The terminal session relay stopped");
        })
    }
}

/// The position to start reading after, retrying until the database answers.
///
/// `None` once `cancel` fires. Keeps retrying instead of falling back to 0,
/// which [`TerminalRelayWorker::spawn`] explains it must not start from.
async fn head(pool: &UiRpcDbPool, cancel: &CancellationToken) -> Option<i64> {
    loop {
        tokio::select! {
            () = cancel.cancelled() => return None,
            result = feed::current_position(pool) => match result {
                Ok(position) => return Some(position),
                Err(error) => warn!(
                    %error,
                    "The terminal session relay could not read the feed head; retrying"
                ),
            },
        }
        tokio::select! {
            () = cancel.cancelled() => return None,
            () = sleep(HEAD_RETRY_BACKOFF) => {}
        }
    }
}
