//! Watcher observation classification.
//!
//! Watchers classify every observation against desired state **and the
//! correlation marker** into exactly one of four cases, each of which either
//! publishes or suppresses. Lifecycle owns operation-driven events; the
//! watcher owns exactly one desired-matching observation — `Ready` while
//! desired Running (the convergence event). Everything else that matches
//! desired state is an echo of something lifecycle already announced.

use moatless_types::sandbox::SandboxDesiredState;

use super::types::{CorrelationMarker, OperationGeneration};

/// What a watcher observed about a pod/container, normalized.
///
/// Hard failures (`Crash`, `OomKilled`, `Evicted`) are **never** expected and
/// always publish, marker or not. Docker reports a force-removed superseded
/// container as a non-zero stop, so that transition remains a correlation
/// candidate until its generation proves whether it was the old container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedTransition {
    /// Sandbox container became ready.
    Ready,
    /// Graceful stop / scaled to 0 / clean exit (exit 0).
    CleanStop,
    /// Docker non-zero stop whose generation may identify deliberate teardown.
    NonZeroStop,
    /// Pod/container removed.
    Deleted,
    /// Non-zero exit / crash-loop. Always a real failure.
    Crash,
    /// OOM kill. Always a real failure.
    OomKilled,
    /// Node-pressure eviction. Always a real failure.
    Evicted,
}

impl ObservedTransition {
    /// Hard failures are never expected and always publish.
    fn is_hard_failure(self) -> bool {
        matches!(self, Self::Crash | Self::OomKilled | Self::Evicted)
    }
}

/// A single watcher observation carrying the *observed* generation stamped on
/// the resource (K8s pod-template label / Docker container label). `None` for
/// a resource with no generation label; these observations are un-correlatable
/// and never suppressed as a superseded echo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub transition: ObservedTransition,
    pub observed_generation: Option<OperationGeneration>,
}

impl Observation {
    pub fn new(
        transition: ObservedTransition,
        observed_generation: Option<OperationGeneration>,
    ) -> Self {
        Self {
            transition,
            observed_generation,
        }
    }
}

/// The four mutually-exclusive classification cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// Observed Ready while desired Running — the one desired-matching
    /// observation that is published (the watcher's convergence event).
    Convergence,
    /// A terminal observation matching desired state that lifecycle already
    /// announced (observed stop while desired Stopped). Suppressed.
    OperationEcho,
    /// An observation the marked operation's `kind` declares expected — the
    /// stop/deletion of the **superseded** pod during restart/redeploy,
    /// identified by an observed generation *below* the marker generation.
    /// Suppressed.
    ExpectedIntermediate,
    /// Everything else (crash, OOM, eviction, external stop while desired
    /// Running). Published.
    Divergence,
}

/// Watcher decision plus the resource generation used for a deterministic
/// convergence event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassificationResult {
    pub classification: Classification,
    pub event_generation: Option<OperationGeneration>,
}

impl Classification {
    /// Convergence and Divergence publish; echoes and expected intermediates
    /// are suppressed.
    pub fn publishes(self) -> bool {
        matches!(self, Self::Convergence | Self::Divergence)
    }
}

/// Classify and select the generation used by a published observation.
///
/// Runtime adapters stamp every replacement, including waking a stopped Docker
/// sandbox, so the observed label is a time-independent event identity.
pub fn classify_with_correlation(
    obs: Observation,
    desired: SandboxDesiredState,
    marker: Option<CorrelationMarker>,
    suppression_active: bool,
) -> ClassificationResult {
    let classification = classify(obs, desired, marker, suppression_active);
    let event_generation = obs.observed_generation;
    ClassificationResult {
        classification,
        event_generation,
    }
}

/// Classify one observation.
///
/// `suppression_active` is the reconciler-managed flag on the marker: a stale
/// edge marker first **disables** suppression (so a lingering unfinished edge
/// cannot indefinitely hide observations), then is cleared only after the
/// replacement generation is observed Ready or the edge is durably abandoned.
/// When `false`, the expected-intermediate suppression does not apply.
pub fn classify(
    obs: Observation,
    desired: SandboxDesiredState,
    marker: Option<CorrelationMarker>,
    suppression_active: bool,
) -> Classification {
    use ObservedTransition as T;

    // 1. Hard failures always publish, marker or not — OOM kills, evictions,
    //    and crash exit codes are never expected.
    if obs.transition.is_hard_failure() {
        return Classification::Divergence;
    }

    match obs.transition {
        // 2. The single published desired-matching observation.
        T::Ready => {
            if desired == SandboxDesiredState::Running {
                Classification::Convergence
            } else {
                // Ready while desired Stopped/Removed is unexpected — publish.
                Classification::Divergence
            }
        }
        T::CleanStop | T::NonZeroStop | T::Deleted => {
            // 3. Expected intermediate: the superseded pod's deliberate death
            //    during a restart/redeploy edge. Identity — not the clock —
            //    distinguishes the old pod's death (generation < marker) from
            //    the new pod's crash (generation == marker), so a stale edge
            //    with disabled suppression falls through to the echo/divergence
            //    rules below.
            if suppression_active {
                if let Some(m) = marker {
                    if m.kind.is_edge() && is_superseded(obs.observed_generation, m.generation) {
                        return Classification::ExpectedIntermediate;
                    }
                }
            }

            // 4. Operation echo vs divergence, arbitrated by desired state.
            match desired {
                // lifecycle already announced Stopped.
                SandboxDesiredState::Stopped if obs.transition == T::CleanStop => {
                    Classification::OperationEcho
                }
                // lifecycle already announced Removed; both a clean stop and a
                // deletion are echoes of the announced removal.
                SandboxDesiredState::Removed => Classification::OperationEcho,
                // External stop while desired Running (or a deletion while
                // desired Stopped) — publish.
                _ => Classification::Divergence,
            }
        }
        // Hard failures handled above.
        T::Crash | T::OomKilled | T::Evicted => Classification::Divergence,
    }
}

/// An observed generation strictly below the marker generation proves the
/// event belongs to a superseded operation. A missing observed generation
/// cannot be proven superseded, so it is not suppressed.
fn is_superseded(observed: Option<OperationGeneration>, marker: OperationGeneration) -> bool {
    matches!(observed, Some(g) if g < marker)
}

#[cfg(test)]
mod tests {
    use super::ObservedTransition as T;
    use super::*;
    use chrono::{TimeZone, Utc};
    use moatless_types::sandbox::SandboxDesiredState as D;

    use crate::lifecycle::types::OperationKind;

    fn marker(kind: OperationKind, gen: i64) -> CorrelationMarker {
        CorrelationMarker::new(
            kind,
            OperationGeneration(gen),
            Utc.timestamp_opt(1_700_000_000, 0)
                .single()
                .expect("valid ts"),
        )
    }

    fn obs(t: T, gen: Option<i64>) -> Observation {
        Observation::new(t, gen.map(OperationGeneration))
    }

    // ---- Case 1: Convergence ----

    #[test]
    fn ready_while_running_is_convergence_and_publishes() {
        let c = classify(
            obs(T::Ready, Some(5)),
            D::Running,
            Some(marker(OperationKind::Start, 5)),
            true,
        );
        assert_eq!(c, Classification::Convergence);
        assert!(c.publishes());
    }

    #[test]
    fn ready_identity_does_not_change_when_the_marker_clears() {
        let result = classify_with_correlation(
            obs(T::Ready, Some(4)),
            D::Running,
            Some(marker(OperationKind::Start, 5)),
            false,
        );
        assert_eq!(result.classification, Classification::Convergence);
        assert_eq!(result.event_generation, Some(OperationGeneration(4)));
        let after_ack = classify_with_correlation(obs(T::Ready, Some(4)), D::Running, None, false);
        assert_eq!(after_ack.event_generation, result.event_generation);
    }

    #[test]
    fn superseded_ready_does_not_take_an_edge_markers_generation() {
        let result = classify_with_correlation(
            obs(T::Ready, Some(4)),
            D::Running,
            Some(marker(OperationKind::Restart, 5)),
            true,
        );
        assert_eq!(result.classification, Classification::Convergence);
        assert_eq!(result.event_generation, Some(OperationGeneration(4)));
    }

    // ---- Case 2: Operation echo ----

    #[test]
    fn clean_stop_while_stopped_is_echo_and_suppressed() {
        let c = classify(
            obs(T::CleanStop, Some(3)),
            D::Stopped,
            Some(marker(OperationKind::Stop, 3)),
            true,
        );
        assert_eq!(c, Classification::OperationEcho);
        assert!(!c.publishes());
    }

    #[test]
    fn deletion_while_removed_is_echo_and_suppressed() {
        let c = classify(
            obs(T::Deleted, Some(4)),
            D::Removed,
            Some(marker(OperationKind::Remove, 4)),
            true,
        );
        assert_eq!(c, Classification::OperationEcho);
        assert!(!c.publishes());
    }

    // ---- Case 3: Expected intermediate (fresh edge marker, superseded gen) ----

    #[test]
    fn superseded_pod_stop_during_restart_is_expected_intermediate() {
        // Restart bumped the generation to 7; the old pod (gen 6) is stopping.
        let c = classify(
            obs(T::CleanStop, Some(6)),
            D::Running,
            Some(marker(OperationKind::Restart, 7)),
            true,
        );
        assert_eq!(c, Classification::ExpectedIntermediate);
        assert!(!c.publishes());
    }

    #[test]
    fn superseded_pod_deletion_during_redeploy_is_expected_intermediate() {
        let c = classify(
            obs(T::Deleted, Some(9)),
            D::Running,
            Some(marker(OperationKind::Redeploy, 10)),
            true,
        );
        assert_eq!(c, Classification::ExpectedIntermediate);
    }

    #[test]
    fn new_pod_crash_during_restart_is_divergence_not_suppressed() {
        // The new pod carries the marker generation (7); a crash on it is a
        // real divergence, not the old pod's expected death.
        let c = classify(
            obs(T::Crash, Some(7)),
            D::Running,
            Some(marker(OperationKind::Restart, 7)),
            true,
        );
        assert_eq!(c, Classification::Divergence);
        assert!(c.publishes());
    }

    #[test]
    fn restart_deliberate_stop_is_not_a_crash() {
        // The whole point of case 3: a restart's deliberate pod stop must not
        // read as a crash. Same superseded stop, asserted to NOT diverge.
        let c = classify(
            obs(T::CleanStop, Some(6)),
            D::Running,
            Some(marker(OperationKind::Restart, 7)),
            true,
        );
        assert_ne!(c, Classification::Divergence);
    }

    #[test]
    fn superseded_docker_nonzero_stop_during_restart_is_expected() {
        let c = classify(
            obs(T::NonZeroStop, Some(6)),
            D::Running,
            Some(marker(OperationKind::Restart, 7)),
            true,
        );
        assert_eq!(c, Classification::ExpectedIntermediate);
        assert!(!c.publishes());
    }

    #[test]
    fn replacement_docker_nonzero_stop_is_a_divergence() {
        let c = classify(
            obs(T::NonZeroStop, Some(7)),
            D::Running,
            Some(marker(OperationKind::Restart, 7)),
            true,
        );
        assert_eq!(c, Classification::Divergence);
        assert!(c.publishes());
    }

    // ---- Case 4: Divergence ----

    #[test]
    fn external_stop_while_running_is_divergence() {
        let c = classify(obs(T::CleanStop, Some(2)), D::Running, None, true);
        assert_eq!(c, Classification::Divergence);
        assert!(c.publishes());
    }

    #[test]
    fn oom_eviction_crash_always_publish_even_under_edge_marker() {
        // A superseded generation must NOT hide a hard failure.
        for t in [T::OomKilled, T::Evicted, T::Crash] {
            let c = classify(
                obs(t, Some(6)),
                D::Running,
                Some(marker(OperationKind::Restart, 7)),
                true,
            );
            assert_eq!(c, Classification::Divergence, "{t:?}");
            assert!(c.publishes(), "{t:?}");
        }
    }

    #[test]
    fn hard_failure_publishes_even_while_stopped() {
        // OOM/crash are real even when desired Stopped — never an echo.
        let c = classify(
            obs(T::OomKilled, Some(3)),
            D::Stopped,
            Some(marker(OperationKind::Stop, 3)),
            true,
        );
        assert_eq!(c, Classification::Divergence);
    }

    // ---- Superseded-identity edge cases ----

    #[test]
    fn level_op_marker_does_not_grant_expected_intermediate() {
        // Only edge operations declare pod stop/deletion expected. A stop
        // marker with a superseded generation does not suppress a Running echo.
        let c = classify(
            obs(T::CleanStop, Some(6)),
            D::Running,
            Some(marker(OperationKind::Stop, 7)),
            true,
        );
        assert_eq!(c, Classification::Divergence);
    }

    #[test]
    fn missing_observed_generation_is_never_suppressed_as_superseded() {
        // Cannot prove superseded without a generation label → publish.
        let c = classify(
            obs(T::CleanStop, None),
            D::Running,
            Some(marker(OperationKind::Restart, 7)),
            true,
        );
        assert_eq!(c, Classification::Divergence);
    }

    #[test]
    fn same_generation_stop_is_not_superseded() {
        // observed == marker (not <) is the current pod, not the old one.
        let c = classify(
            obs(T::CleanStop, Some(7)),
            D::Running,
            Some(marker(OperationKind::Restart, 7)),
            true,
        );
        assert_eq!(c, Classification::Divergence);
    }

    // ---- Stale edge marker disables suppression ----

    #[test]
    fn stale_edge_marker_with_disabled_suppression_publishes_old_stop() {
        // Once the reconciler flags the edge stale, suppression is off; the
        // same superseded stop now falls through to divergence and publishes.
        let c = classify(
            obs(T::CleanStop, Some(6)),
            D::Running,
            Some(marker(OperationKind::Restart, 7)),
            false,
        );
        assert_eq!(c, Classification::Divergence);
        assert!(c.publishes());
    }
}
