use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Liveness/readiness view of supervised work, read by a process's own probe.
///
/// Records every task whose [`Policy`](crate::Policy) chose to record it as
/// failed. The set only grows: nothing here is restartable, so a process that
/// has lost one of these stays degraded until it is replaced.
#[derive(Debug, Default)]
pub struct SupervisorHealth {
    failed: Mutex<BTreeSet<&'static str>>,
}

impl SupervisorHealth {
    /// Names of tasks recorded as failed, in name order. Empty means nothing
    /// tracked here has terminated early.
    pub fn failed_tasks(&self) -> Vec<&'static str> {
        self.guard().iter().copied().collect()
    }

    /// Record a task as failed. The only caller is [`classify`](crate::classify),
    /// which decides this from the [`Policy`](crate::Policy)'s
    /// [`Outcome`](crate::Outcome) — kept `pub(crate)` so that stays the one
    /// writer.
    pub(crate) fn record(&self, name: &'static str) {
        self.guard().insert(name);
    }

    /// A poisoned lock only means a writer panicked mid-insert; the set is
    /// still a valid liveness view, and reporting *less* failure than reality
    /// is the wrong way to fail.
    fn guard(&self) -> MutexGuard<'_, BTreeSet<&'static str>> {
        self.failed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
