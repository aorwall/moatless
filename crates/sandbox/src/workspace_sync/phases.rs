//! Where one upload's time went, readable while the upload is still running.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The phases one upload has entered, each stamped with when it began.
///
/// Shared with the blocking thread `restic::snapshot` runs on, so the shutdown
/// flush can still name the phase its budget ran out in after it has dropped
/// the upload's future.
pub(super) struct Phases {
    started: Instant,
    entered: Mutex<Vec<(&'static str, Duration)>>,
}

impl Phases {
    pub(super) fn start() -> Arc<Self> {
        Arc::new(Self {
            started: Instant::now(),
            entered: Mutex::new(Vec::new()),
        })
    }

    /// Mark `phase` as begun, which ends the one before it.
    pub(super) fn enter(&self, phase: &'static str) {
        let at = self.started.elapsed();
        self.lock().push((phase, at));
    }

    /// Time since [`Self::start`].
    pub(super) fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// The phase running now, or `None` before the first [`Self::enter`].
    pub(super) fn current(&self) -> Option<&'static str> {
        self.lock().last().map(|(phase, _)| *phase)
    }

    /// Each phase with how long it ran, the last one up to now:
    /// `lock 0.0s, fingerprint 0.3s, backup 7.2s`.
    pub(super) fn summary(&self) -> String {
        let now = self.started.elapsed();
        let entered = self.lock();
        let ends = entered.iter().skip(1).map(|(_, at)| *at).chain([now]);
        entered
            .iter()
            .zip(ends)
            .map(|((phase, began), ended)| {
                format!("{phase} {:.1}s", ended.saturating_sub(*began).as_secs_f64())
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(&'static str, Duration)>> {
        self.entered.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_phase_runs_until_the_next_begins() {
        let phases = Phases::start();
        assert_eq!(phases.current(), None);
        assert_eq!(phases.summary(), "");

        phases.enter("lock");
        phases.enter("backup");

        assert_eq!(phases.current(), Some("backup"));
        let summary = phases.summary();
        assert!(summary.starts_with("lock 0.0s, backup "), "{summary}");
    }
}
