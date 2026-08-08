//! The reconciler's one normalized convergence definition.
//!
//! The reconciler compares [`SandboxDesiredState`] against
//! [`SandboxObservedStatus`] — **never bare "state"** — and every non-converged
//! pairing has a rule. There is no unmapped pairing.
//!
//! Archive deletion is part of strict remove *success*, not of observed
//! convergence — the remove marker carries that obligation. `remove_marker_incomplete`
//! is the one input that lets an all-absent Removed target still demand a
//! retry, because the archive-deletion obligation has no observable trace.

use moatless_tasks::task::models::SandboxDesiredState;

pub use moatless_events::SandboxStatus;

/// The reconciler's normalized observed vocabulary.
///
/// Distinct from the display [`SandboxStatus`] (which mixes intent-derived
/// values like `Removed`) and from the operation-level
/// [`super::types::ObservedStatus`]. `Absent` means **every runtime resource
/// is gone, PVC unconditionally included** — the reconciler treats an existing
/// PVC as not-absent. `Terminating` is accepted-but-still-terminating: a
/// delete was accepted but resources persist, which is *not* converged for
/// Removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxObservedStatus {
    Ready,
    Starting,
    Stopped,
    /// Every runtime resource gone (Deployment, Service, IngressRoutes, PVC).
    Absent,
    /// A terminal/error pod — crashed, failed, or OOM. `ensure_running`
    /// replaces it rather than treating it as indefinitely Initializing.
    Error,
    /// Accepted deletion still draining — resources present but terminating.
    Terminating,
}

impl SandboxObservedStatus {
    /// Normalize the display [`SandboxStatus`] projection into the reconciler's
    /// observed vocabulary.
    ///
    /// `Removed` is intent-derived, not an observation — a reconciler never
    /// sees it from the runtime; it is mapped to `Absent` defensively.
    pub fn from_display(status: SandboxStatus) -> Self {
        match status {
            SandboxStatus::Ready => Self::Ready,
            SandboxStatus::Initializing => Self::Starting,
            SandboxStatus::Stopped => Self::Stopped,
            SandboxStatus::NotCreated | SandboxStatus::Removed => Self::Absent,
            SandboxStatus::Removing => Self::Terminating,
            SandboxStatus::Error => Self::Error,
        }
    }
}

/// The action the reconciler drives — **always through lifecycle operations,
/// never directly**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileAction {
    /// Converged — nothing to do.
    None,
    /// Non-converged but progressing (Starting; a superseded pod draining).
    /// Remediated by the pending-timeout pass after its deadline, not by a
    /// fresh operation this tick.
    Wait,
    Start,
    Stop,
    Remove,
}

impl ReconcileAction {
    /// A concrete lifecycle operation to drive this tick (not `None`/`Wait`).
    pub fn is_operation(self) -> bool {
        matches!(self, Self::Start | Self::Stop | Self::Remove)
    }
}

/// Is the observed status converged on the desired state?
///
/// - Running ↔ `Ready`
/// - Stopped ↔ `Stopped` **or** `Absent` (a missing runtime satisfies Stopped)
/// - Removed ↔ `Absent` (every resource gone, PVC included) — and only when the
///   remove marker's archive obligation is complete.
///
/// The driver acts through [`reconcile_action`]; this is the paired predicate it
/// is verified total against (action is `None` exactly when converged), so it is
/// exercised only by tests.
#[cfg(test)]
pub fn converged(
    desired: SandboxDesiredState,
    observed: SandboxObservedStatus,
    remove_marker_incomplete: bool,
) -> bool {
    use SandboxObservedStatus as O;
    match desired {
        SandboxDesiredState::Running => observed == O::Ready,
        SandboxDesiredState::Stopped => matches!(observed, O::Stopped | O::Absent),
        // An incomplete remove marker means archive deletion has not completed;
        // absence alone does not prove convergence.
        SandboxDesiredState::Removed => observed == O::Absent && !remove_marker_incomplete,
    }
}

/// The reconciler decision for a `(desired, observed)` pair.
///
/// `remove_marker_incomplete` is load-bearing when runtime resources are absent
/// but archive deletion has not completed: it keeps the reconciler retrying
/// `remove` even when no resources are live.
pub fn reconcile_action(
    desired: SandboxDesiredState,
    observed: SandboxObservedStatus,
    remove_marker_incomplete: bool,
) -> ReconcileAction {
    use ReconcileAction as A;
    use SandboxObservedStatus as O;

    match desired {
        SandboxDesiredState::Running => match observed {
            O::Ready => A::None,
            // Wait (pending-timeout remediates); a draining superseded pod is
            // heading toward Absent, then Start.
            O::Starting | O::Terminating => A::Wait,
            // ensure_running replaces a terminal/error pod from the fresh plan
            // rather than treating it as Initializing.
            O::Stopped | O::Absent | O::Error => A::Start,
        },
        SandboxDesiredState::Stopped => match observed {
            O::Stopped | O::Absent => A::None,
            // Already draining toward a satisfying (Absent) state.
            O::Terminating => A::Wait,
            O::Ready | O::Starting | O::Error => A::Stop,
        },
        SandboxDesiredState::Removed => match observed {
            // Absence with a complete obligation is converged; with an
            // incomplete marker the archive obligation forces a retry.
            O::Absent => {
                if remove_marker_incomplete {
                    A::Remove
                } else {
                    A::None
                }
            }
            // Every live status → remove (idempotent; the remove op awaits
            // absence). Terminating is not-yet-absent, so re-drive remove to
            // guarantee progress after a crash.
            O::Ready | O::Starting | O::Stopped | O::Error | O::Terminating => A::Remove,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::SandboxObservedStatus as O;
    use super::*;
    use moatless_tasks::task::models::SandboxDesiredState as D;

    const ALL_OBSERVED: [O; 6] = [
        O::Ready,
        O::Starting,
        O::Stopped,
        O::Absent,
        O::Error,
        O::Terminating,
    ];
    const ALL_DESIRED: [D; 3] = [D::Running, D::Stopped, D::Removed];

    // ---- Convergence matrix (Running) ----

    #[test]
    fn running_converges_only_on_ready() {
        for o in ALL_OBSERVED {
            let want = o == O::Ready;
            assert_eq!(converged(D::Running, o, false), want, "observed {o:?}");
        }
    }

    #[test]
    fn running_starts_from_stopped_missing_and_error() {
        assert_eq!(
            reconcile_action(D::Running, O::Stopped, false),
            ReconcileAction::Start
        );
        assert_eq!(
            reconcile_action(D::Running, O::Absent, false),
            ReconcileAction::Start
        );
        // Running/error repairs through ensure_running.
        assert_eq!(
            reconcile_action(D::Running, O::Error, false),
            ReconcileAction::Start
        );
    }

    #[test]
    fn running_waits_on_starting_then_times_out() {
        assert_eq!(
            reconcile_action(D::Running, O::Starting, false),
            ReconcileAction::Wait
        );
    }

    #[test]
    fn running_ready_is_no_action() {
        assert_eq!(
            reconcile_action(D::Running, O::Ready, false),
            ReconcileAction::None
        );
    }

    // ---- Convergence matrix (Stopped) ----

    #[test]
    fn stopped_converges_on_stopped_or_absent() {
        assert!(converged(D::Stopped, O::Stopped, false));
        // A missing runtime satisfies Stopped.
        assert!(converged(D::Stopped, O::Absent, false));
        assert!(!converged(D::Stopped, O::Ready, false));
        assert!(!converged(D::Stopped, O::Starting, false));
        assert!(!converged(D::Stopped, O::Error, false));
    }

    #[test]
    fn stopped_stops_any_live_status() {
        assert_eq!(
            reconcile_action(D::Stopped, O::Ready, false),
            ReconcileAction::Stop
        );
        assert_eq!(
            reconcile_action(D::Stopped, O::Starting, false),
            ReconcileAction::Stop
        );
        assert_eq!(
            reconcile_action(D::Stopped, O::Error, false),
            ReconcileAction::Stop
        );
        assert_eq!(
            reconcile_action(D::Stopped, O::Stopped, false),
            ReconcileAction::None
        );
        assert_eq!(
            reconcile_action(D::Stopped, O::Absent, false),
            ReconcileAction::None
        );
    }

    // ---- Convergence matrix (Removed) + removal obligation ----

    #[test]
    fn removed_converges_only_on_absent_with_complete_marker() {
        // Absent + complete obligation → converged.
        assert!(converged(D::Removed, O::Absent, false));
        // Absent + incomplete marker → NOT converged (archive obligation).
        assert!(!converged(D::Removed, O::Absent, true));
        // Any live status → never converged.
        for o in [O::Ready, O::Starting, O::Stopped, O::Error, O::Terminating] {
            assert!(!converged(D::Removed, o, false), "observed {o:?}");
        }
    }

    #[test]
    fn removed_absent_incomplete_marker_retries_remove() {
        // The load-bearing marker branch: no resources live, but the archive
        // obligation is unmet, so the reconciler retries remove.
        assert_eq!(
            reconcile_action(D::Removed, O::Absent, true),
            ReconcileAction::Remove
        );
        // Complete obligation → nothing to do.
        assert_eq!(
            reconcile_action(D::Removed, O::Absent, false),
            ReconcileAction::None
        );
    }

    #[test]
    fn removed_removes_every_live_status() {
        for o in [O::Ready, O::Starting, O::Stopped, O::Error, O::Terminating] {
            assert_eq!(
                reconcile_action(D::Removed, o, false),
                ReconcileAction::Remove,
                "observed {o:?}"
            );
        }
    }

    // ---- Total coverage: no unmapped pairing ----

    #[test]
    fn every_pairing_has_a_rule_and_none_panics() {
        for d in ALL_DESIRED {
            for o in ALL_OBSERVED {
                for incomplete in [false, true] {
                    let action = reconcile_action(d, o, incomplete);
                    // A converged pair must map to None (or, for Removed with an
                    // incomplete obligation, Remove). A non-converged pair must
                    // never be None.
                    if converged(d, o, incomplete) {
                        assert_eq!(action, ReconcileAction::None, "{d:?}/{o:?}/{incomplete}");
                    } else {
                        assert_ne!(action, ReconcileAction::None, "{d:?}/{o:?}/{incomplete}");
                    }
                }
            }
        }
    }

    #[test]
    fn accepted_but_terminating_removed_is_not_converged() {
        // Kubernetes delete returns on acceptance; terminating is not absence.
        assert!(!converged(D::Removed, O::Terminating, false));
        assert_eq!(
            reconcile_action(D::Removed, O::Terminating, false),
            ReconcileAction::Remove
        );
    }

    #[test]
    fn display_status_normalizes_into_observed_vocab() {
        assert_eq!(
            SandboxObservedStatus::from_display(SandboxStatus::Ready),
            O::Ready
        );
        assert_eq!(
            SandboxObservedStatus::from_display(SandboxStatus::Initializing),
            O::Starting
        );
        assert_eq!(
            SandboxObservedStatus::from_display(SandboxStatus::Stopped),
            O::Stopped
        );
        assert_eq!(
            SandboxObservedStatus::from_display(SandboxStatus::NotCreated),
            O::Absent
        );
        assert_eq!(
            SandboxObservedStatus::from_display(SandboxStatus::Removed),
            O::Absent
        );
        assert_eq!(
            SandboxObservedStatus::from_display(SandboxStatus::Removing),
            O::Terminating
        );
        assert_eq!(
            SandboxObservedStatus::from_display(SandboxStatus::Error),
            O::Error
        );
    }
}
