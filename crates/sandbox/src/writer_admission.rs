//! Admission for everything this pod can start that writes the workspace.
//!
//! One owner for the pod: teardown closes it once, ahead of stopping any
//! writer, and waits out the requests it had already admitted. A capture is
//! certified as a pod's final tree only over a workspace that was closed and
//! drained first (`workspace_sync::sync::workspace_writers_are_quiet`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::watch;
use tracing::warn;

/// What [`WriterAdmission::close_and_drain`] found once it had closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drain {
    /// Every request admitted before the close has finished.
    Complete,
    /// One outlasted the budget, so it may still start a writer.
    TimedOut,
}

/// Admission state shared by every workspace-writer entry point.
pub struct WriterAdmission {
    closed: AtomicBool,
    drained: AtomicBool,
    /// Requests admitted and not yet finished. A `watch` so the drain waits on
    /// a change rather than polling.
    in_flight: watch::Sender<usize>,
}

impl Default for WriterAdmission {
    fn default() -> Self {
        Self::new()
    }
}

impl WriterAdmission {
    pub fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            drained: AtomicBool::new(false),
            in_flight: watch::Sender::new(0),
        }
    }

    /// A permit for one workspace writer, or `None` once teardown has begun.
    ///
    /// Hold it until the writer is one the shutdown flush can observe — the
    /// command registered, the file written. Dropping it earlier lets
    /// [`Self::close_and_drain`] report a tree nobody is writing while this
    /// writer has yet to start.
    pub fn admit(self: &Arc<Self>) -> Option<WriterPermit> {
        // Counted before the flag is read, while the close stores the flag
        // before it reads the count: whichever order the two land in, one of
        // them sees the other.
        self.in_flight.send_modify(|count| *count += 1);
        let permit = WriterPermit {
            admission: Arc::clone(self),
        };
        (!self.closed.load(Ordering::SeqCst)).then_some(permit)
    }

    /// Whether [`Self::close_and_drain`] has run and found nothing outstanding.
    pub fn is_drained(&self) -> bool {
        self.closed.load(Ordering::SeqCst) && self.drained.load(Ordering::SeqCst)
    }

    /// Whether [`Self::close_and_drain`] has run at all.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Refuse every further writer, then wait out the admitted ones.
    ///
    /// Never reopened: a `moat cmd` child is spawned `kill_on_drop(false)` and
    /// writes until the grace period's SIGKILL, so one admitted after teardown
    /// has read the tree outlives every later check of it.
    pub async fn close_and_drain(&self, budget: Duration) -> Drain {
        self.closed.store(true, Ordering::SeqCst);
        if tokio::time::timeout(budget, self.wait_until_idle())
            .await
            .is_err()
        {
            warn!("Shutdown: a request admitted before the close outlasted the drain");
            return Drain::TimedOut;
        }
        self.drained.store(true, Ordering::SeqCst);
        Drain::Complete
    }

    async fn wait_until_idle(&self) {
        let mut in_flight = self.in_flight.subscribe();
        while *in_flight.borrow_and_update() > 0 {
            if in_flight.changed().await.is_err() {
                return;
            }
        }
    }
}

/// One admitted writer, counted until it drops. See [`WriterAdmission::admit`].
pub struct WriterPermit {
    admission: Arc<WriterAdmission>,
}

impl Drop for WriterPermit {
    fn drop(&mut self) {
        self.admission
            .in_flight
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_closed_admission_refuses_every_further_writer() {
        let admission = Arc::new(WriterAdmission::new());
        assert!(admission.admit().is_some());

        assert_eq!(
            admission.close_and_drain(Duration::from_secs(5)).await,
            Drain::Complete
        );
        assert!(admission.admit().is_none());
        assert!(admission.is_drained());
    }

    /// The race the flag alone lost: a request that passed admission before the
    /// close, and has yet to start its writer, holds the drain open.
    #[tokio::test]
    async fn a_writer_admitted_before_the_close_holds_the_drain_open() {
        let admission = Arc::new(WriterAdmission::new());
        let permit = admission.admit().expect("open");

        let draining = tokio::spawn({
            let admission = Arc::clone(&admission);
            async move { admission.close_and_drain(Duration::from_secs(30)).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !draining.is_finished(),
            "the drain finished while an admitted writer had not"
        );
        assert!(!admission.is_drained());

        drop(permit);
        assert_eq!(draining.await.expect("join"), Drain::Complete);
        assert!(admission.is_drained());
    }

    #[tokio::test(start_paused = true)]
    async fn a_writer_that_outlasts_the_budget_leaves_the_tree_uncertifiable() {
        let admission = Arc::new(WriterAdmission::new());
        let _permit = admission.admit().expect("open");

        assert_eq!(
            admission.close_and_drain(Duration::from_millis(50)).await,
            Drain::TimedOut
        );
        assert!(admission.is_closed());
        assert!(
            !admission.is_drained(),
            "a writer that outlasted the drain must deny the final-tree tag"
        );
    }
}
