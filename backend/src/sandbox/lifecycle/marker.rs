//! Correlation-marker acknowledgement — the reconciler's per-class rules.
//!
//! The reconciler is the marker's acknowledgement owner. Marker age is a
//! recovery signal, **never** proof of completion: stale level markers are
//! reconciled and never cleared merely because of age; an incomplete remove
//! marker survives until strict success. A stale edge marker first disables
//! event suppression, then is cleared only after the replacement generation is
//! observed Ready or the edge is durably classified as abandoned with its
//! deterministic `Error` fact persisted.

use std::time::Duration;

use chrono::{DateTime, Utc};

use super::convergence::SandboxObservedStatus;
use super::types::{CorrelationMarker, OperationKind};

/// An edge older than this bound no longer suppresses watcher observations.
pub const EDGE_SUPPRESSION_STALE_AFTER: Duration = Duration::from_mins(5);

/// An edge older than this second bound may be durably abandoned once the
/// replacement generation is no longer observed making progress.
pub const EDGE_ABANDON_AFTER: Duration = Duration::from_mins(15);

/// The reconciler's decision for a persisted marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerAck {
    /// Acknowledge and clear the marker.
    Clear,
    /// Keep the marker — recovery is still pending.
    Retain,
}

impl MarkerAck {
    pub fn clears(self) -> bool {
        matches!(self, Self::Clear)
    }
}

/// Inputs the reconciler gathers to acknowledge a marker.
#[derive(Debug, Clone, Copy)]
pub struct AckContext {
    /// Normalized observed status.
    pub observed: SandboxObservedStatus,
    /// Edge ops: the **replacement** generation has been observed Ready.
    pub replacement_ready: bool,
    /// Stop/remove: the deterministic completion fact is durably persisted.
    /// For remove this implies strict success including archive deletion,
    /// because the completion fact is only written on strict success.
    pub completion_persisted: bool,
    /// Remove: every runtime resource is absent (PVC unconditionally included).
    pub resources_absent: bool,
    /// Edge ops: the edge is durably classified as abandoned and its `Error`
    /// fact is persisted.
    pub edge_abandoned_error_persisted: bool,
}

impl Default for AckContext {
    fn default() -> Self {
        Self {
            observed: SandboxObservedStatus::Absent,
            replacement_ready: false,
            completion_persisted: false,
            resources_absent: false,
            edge_abandoned_error_persisted: false,
        }
    }
}

/// Decide whether a persisted marker can be cleared.
///
/// - **start** clears after observed Ready;
/// - **stop** clears after observed convergence **and** durable persistence of
///   its `Stopped` completion event;
/// - **remove** clears only after strict success **and** durable persistence of
///   its `Removed` completion event — observed absence is not enough, because
///   the archive-deletion obligation has no observable trace;
/// - **edge** ops clear only when the replacement generation is observed Ready
///   (or the edge is durably abandoned with its `Error` fact persisted).
pub fn acknowledge(marker: &CorrelationMarker, ctx: AckContext) -> MarkerAck {
    use SandboxObservedStatus as O;
    let clear = match marker.kind {
        OperationKind::Start => ctx.observed == O::Ready,
        OperationKind::Stop => {
            matches!(ctx.observed, O::Stopped | O::Absent) && ctx.completion_persisted
        }
        // Observed absence alone never clears a remove marker: the archive
        // obligation forces waiting for the durable completion fact.
        OperationKind::Remove => ctx.resources_absent && ctx.completion_persisted,
        OperationKind::Restart | OperationKind::Redeploy => {
            ctx.replacement_ready || ctx.edge_abandoned_error_persisted
        }
    };
    if clear {
        MarkerAck::Clear
    } else {
        MarkerAck::Retain
    }
}

/// Whether a marker is stale (age exceeds `threshold`).
///
/// Staleness is a recovery signal only — it is **never** grounds to clear a
/// level or remove marker. For an edge marker it drives disabling suppression.
pub fn is_stale(marker: &CorrelationMarker, now: DateTime<Utc>, threshold: Duration) -> bool {
    let age = now.signed_duration_since(marker.started_at);
    match age.to_std() {
        Ok(age) => age > threshold,
        // Negative age (marker in the future — clock skew) is not stale.
        Err(_) => false,
    }
}

/// Whether the watcher's expected-intermediate suppression is active for this
/// marker. Only edge markers ever suppress; a stale edge marker disables it so
/// a lingering unfinished edge cannot indefinitely hide observations.
pub fn edge_suppression_active(
    marker: &CorrelationMarker,
    now: DateTime<Utc>,
    stale_after: Duration,
) -> bool {
    if !marker.kind.is_edge() {
        // Level markers don't grant expected-intermediate suppression at all,
        // so the concept is vacuously "not suppressing".
        return false;
    }
    !is_stale(marker, now, stale_after)
}

#[cfg(test)]
mod tests {
    use super::SandboxObservedStatus as O;
    use super::*;
    use crate::sandbox::lifecycle::types::OperationGeneration;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().expect("valid ts")
    }

    fn marker(kind: OperationKind) -> CorrelationMarker {
        CorrelationMarker::new(kind, OperationGeneration(1), at(1_000))
    }

    // ---- start ----

    #[test]
    fn start_marker_clears_only_on_ready() {
        let m = marker(OperationKind::Start);
        assert_eq!(
            acknowledge(
                &m,
                AckContext {
                    observed: O::Ready,
                    ..Default::default()
                }
            ),
            MarkerAck::Clear
        );
        for o in [O::Starting, O::Stopped, O::Absent, O::Error, O::Terminating] {
            assert_eq!(
                acknowledge(
                    &m,
                    AckContext {
                        observed: o,
                        ..Default::default()
                    }
                ),
                MarkerAck::Retain,
                "{o:?}"
            );
        }
    }

    // ---- stop ----

    #[test]
    fn stop_marker_needs_convergence_and_persisted_completion() {
        let m = marker(OperationKind::Stop);
        // Converged but completion not yet persisted → retain.
        assert_eq!(
            acknowledge(
                &m,
                AckContext {
                    observed: O::Stopped,
                    completion_persisted: false,
                    ..Default::default()
                }
            ),
            MarkerAck::Retain
        );
        // Converged AND persisted → clear.
        assert_eq!(
            acknowledge(
                &m,
                AckContext {
                    observed: O::Stopped,
                    completion_persisted: true,
                    ..Default::default()
                }
            ),
            MarkerAck::Clear
        );
        // Absent also satisfies stop convergence.
        assert_eq!(
            acknowledge(
                &m,
                AckContext {
                    observed: O::Absent,
                    completion_persisted: true,
                    ..Default::default()
                }
            ),
            MarkerAck::Clear
        );
        // Not converged → retain regardless of persistence.
        assert_eq!(
            acknowledge(
                &m,
                AckContext {
                    observed: O::Ready,
                    completion_persisted: true,
                    ..Default::default()
                }
            ),
            MarkerAck::Retain
        );
    }

    // ---- remove (the load-bearing archive-obligation case) ----

    #[test]
    fn remove_marker_survives_observed_absence_without_persisted_completion() {
        let m = marker(OperationKind::Remove);
        // Resources absent but completion (archive obligation) not persisted →
        // retain. Observed absence is NOT enough.
        assert_eq!(
            acknowledge(
                &m,
                AckContext {
                    resources_absent: true,
                    completion_persisted: false,
                    ..Default::default()
                }
            ),
            MarkerAck::Retain
        );
        // Strict success (absent + persisted) → clear.
        assert_eq!(
            acknowledge(
                &m,
                AckContext {
                    resources_absent: true,
                    completion_persisted: true,
                    ..Default::default()
                }
            ),
            MarkerAck::Clear
        );
        // Resources still present → retain even if a stray persist flag is set.
        assert_eq!(
            acknowledge(
                &m,
                AckContext {
                    resources_absent: false,
                    completion_persisted: true,
                    ..Default::default()
                }
            ),
            MarkerAck::Retain
        );
    }

    #[test]
    fn remove_marker_is_never_cleared_merely_by_age() {
        let m = marker(OperationKind::Remove);
        // Even a very old marker is not stale-cleared: acknowledge takes no age
        // input, and is_stale is only a recovery signal.
        assert!(is_stale(&m, at(1_000_000), Duration::from_secs(60)));
        // Acknowledgement with an unmet obligation still retains.
        assert_eq!(
            acknowledge(
                &m,
                AckContext {
                    resources_absent: true,
                    completion_persisted: false,
                    ..Default::default()
                }
            ),
            MarkerAck::Retain
        );
    }

    // ---- edges ----

    #[test]
    fn edge_marker_clears_only_on_replacement_ready() {
        for kind in [OperationKind::Restart, OperationKind::Redeploy] {
            let m = marker(kind);
            // desired==observed alone (Ready of the superseded pod) must NOT
            // clear — only the replacement generation Ready does.
            assert_eq!(
                acknowledge(
                    &m,
                    AckContext {
                        observed: O::Ready,
                        replacement_ready: false,
                        ..Default::default()
                    }
                ),
                MarkerAck::Retain,
                "{kind:?}"
            );
            assert_eq!(
                acknowledge(
                    &m,
                    AckContext {
                        replacement_ready: true,
                        ..Default::default()
                    }
                ),
                MarkerAck::Clear,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn edge_marker_clears_when_durably_abandoned() {
        let m = marker(OperationKind::Redeploy);
        assert_eq!(
            acknowledge(
                &m,
                AckContext {
                    edge_abandoned_error_persisted: true,
                    ..Default::default()
                }
            ),
            MarkerAck::Clear
        );
    }

    // ---- staleness / suppression ----

    #[test]
    fn edge_suppression_disables_when_stale() {
        let m = marker(OperationKind::Restart);
        // Fresh (age 30s < 60s) → suppression active.
        assert!(edge_suppression_active(
            &m,
            at(1_030),
            Duration::from_secs(60)
        ));
        // Stale (age 120s > 60s) → suppression disabled.
        assert!(!edge_suppression_active(
            &m,
            at(1_120),
            Duration::from_secs(60)
        ));
    }

    #[test]
    fn level_marker_never_grants_suppression() {
        for kind in [
            OperationKind::Start,
            OperationKind::Stop,
            OperationKind::Remove,
        ] {
            let m = marker(kind);
            assert!(
                !edge_suppression_active(&m, at(1_010), Duration::from_secs(60)),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn clock_skew_marker_is_not_stale() {
        let m = marker(OperationKind::Start);
        // now < started_at → negative age → not stale.
        assert!(!is_stale(&m, at(500), Duration::from_secs(60)));
    }
}
