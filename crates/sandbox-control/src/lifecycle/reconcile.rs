//! The desired-state reconciler's per-Task decision.
//!
//! The reconciler is the single async recovery owner for level states. Its
//! decision combines message-wake causal precedence ([`wake`](super::wake)) and
//! the desired/observed convergence map ([`convergence`](super::convergence))
//! into one action the driver issues through a lifecycle operation, never
//! directly. See [`decide`] for how a pending message shifts the effective
//! desired state.

use chrono::{DateTime, Utc};

use moatless_types::sandbox::SandboxDesiredState;

use moatless_sandbox_runtime::convergence::{ReconcileAction, SandboxObservedStatus, reconcile_action};
use super::orchestrator::TaskIntent;
use moatless_sandbox_runtime::operation::OperationKind;
use super::wake::{WakeIntent, message_supersedes_intent};

/// One Task's reconcile inputs: persisted intent, observed runtime status, and
/// the newest unacknowledged user message (the pending-message query's output,
/// `None` when the Task has no pending user message).
#[derive(Debug, Clone)]
pub struct ReconcileInput {
    pub intent: TaskIntent,
    pub observed: SandboxObservedStatus,
    pub newest_pending_message_at: Option<DateTime<Utc>>,
}

/// The reconciler's per-Task decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileDecision {
    /// The convergence action to drive through a lifecycle operation.
    pub action: ReconcileAction,
    /// A causally eligible pending message superseded the persisted intent, so
    /// the effective desired state is Running. The driver resets the attempt
    /// budget on a wake so a stalled Task's retries never starve a fresh wake.
    pub woken: bool,
}

/// A pending message that causally supersedes intent (per [`wake`]) makes the
/// effective desired state Running; the convergence map then chooses the
/// action. A still-present remove marker keeps `remove` retrying past observed
/// absence, since archive-deletion has no observable trace.
pub fn decide(input: &ReconcileInput) -> ReconcileDecision {
    let intent = &input.intent;

    let woken = input.newest_pending_message_at.is_some_and(|at| {
        message_supersedes_intent(
            WakeIntent {
                task_status: intent.status,
                desired_state: intent.desired_state,
                retention: intent.retention,
                desired_state_updated_at: intent.desired_state_updated_at,
            },
            at,
        )
    });

    let effective_desired = if woken {
        SandboxDesiredState::Running
    } else {
        intent.desired_state
    };

    let remove_marker_incomplete = effective_desired == SandboxDesiredState::Removed
        && intent
            .marker
            .as_ref()
            .is_some_and(|m| m.kind == OperationKind::Remove);

    let mut action = reconcile_action(effective_desired, input.observed, remove_marker_incomplete);

    // A Stopped Task that converged on the runtime but still carries its stop
    // marker owes a `Stopped` completion commit (a crash between convergence and
    // the commit). Re-drive stop so the orchestrator's Unchanged+NoOp+incomplete
    // -marker path commits the fact and clears the marker — otherwise the
    // incomplete marker re-enumerates the Task as a permanent no-op every tick.
    if action == ReconcileAction::None
        && effective_desired == SandboxDesiredState::Stopped
        && intent
            .marker
            .as_ref()
            .is_some_and(|m| m.kind == OperationKind::Stop)
    {
        action = ReconcileAction::Stop;
    }

    // Stale edge markers (Restart/Redeploy) are deliberately not handled here.
    // The spec's rule — disable event suppression, then clear only once the
    // replacement generation is observed Ready or the edge is durably classified
    // abandoned with its `Error` fact — needs generation identity, so it lands
    // with generation-label threading (see the PR's remaining-work list).

    ReconcileDecision { action, woken }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    use moatless_types::access_control::Scope;
    use moatless_types::sandbox::SandboxStopReason;
    use moatless_types::task::{TaskStatus, Visibility};

    use moatless_sandbox_runtime::operation::{CorrelationMarker, OperationGeneration};
    use moatless_types::sandbox::ArchiveRetention;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().expect("valid ts")
    }

    fn intent(
        status: TaskStatus,
        desired_state: SandboxDesiredState,
        retention: Option<ArchiveRetention>,
        marker: Option<CorrelationMarker>,
    ) -> TaskIntent {
        TaskIntent {
            status,
            visibility: Visibility::Private,
            scope: Scope::User,
            owner_user_id: Some("user-1".to_string()),
            sandbox_id: "t1".to_string(),
            workspace_id: "ws_repo-x".to_string(),
            desired_state,
            stop_reason: None,
            retention,
            marker,
            desired_state_updated_at: at(1_000),
        }
    }

    fn remove_marker() -> CorrelationMarker {
        CorrelationMarker::new(OperationKind::Remove, OperationGeneration(1), at(1_000))
    }

    fn input(
        intent: TaskIntent,
        observed: SandboxObservedStatus,
        pending: Option<DateTime<Utc>>,
    ) -> ReconcileInput {
        ReconcileInput {
            intent,
            observed,
            newest_pending_message_at: pending,
        }
    }

    // ---- No pending message: plain convergence ----

    #[test]
    fn no_pending_message_uses_persisted_intent() {
        let d = decide(&input(
            intent(TaskStatus::Open, SandboxDesiredState::Running, None, None),
            SandboxObservedStatus::Absent,
            None,
        ));
        assert_eq!(d.action, ReconcileAction::Start);
        assert!(!d.woken);
    }

    #[test]
    fn desired_stopped_but_running_stops() {
        let d = decide(&input(
            intent(TaskStatus::Open, SandboxDesiredState::Stopped, None, None),
            SandboxObservedStatus::Ready,
            None,
        ));
        assert_eq!(d.action, ReconcileAction::Stop);
        assert!(!d.woken);
    }

    // ---- Wake supersedes → effective desired Running ----

    #[test]
    fn newer_message_wakes_a_stopped_task() {
        // Stopped at 1_000; a message at 1_001 supersedes → converge to Running.
        let d = decide(&input(
            intent(TaskStatus::Open, SandboxDesiredState::Stopped, None, None),
            SandboxObservedStatus::Absent,
            Some(at(1_001)),
        ));
        assert_eq!(d.action, ReconcileAction::Start);
        assert!(d.woken);
    }

    #[test]
    fn woken_but_already_ready_is_no_action() {
        let d = decide(&input(
            intent(TaskStatus::Open, SandboxDesiredState::Stopped, None, None),
            SandboxObservedStatus::Ready,
            Some(at(1_001)),
        ));
        assert_eq!(d.action, ReconcileAction::None);
        assert!(d.woken);
    }

    #[test]
    fn older_message_does_not_wake_a_later_stop() {
        // Message at 999 is older than the stop at 1_000 — no supersede, stays Stopped.
        let d = decide(&input(
            intent(TaskStatus::Open, SandboxDesiredState::Stopped, None, None),
            SandboxObservedStatus::Absent,
            Some(at(999)),
        ));
        assert_eq!(d.action, ReconcileAction::None);
        assert!(!d.woken);
    }

    #[test]
    fn closed_task_is_never_woken() {
        let d = decide(&input(
            intent(TaskStatus::Closed, SandboxDesiredState::Stopped, None, None),
            SandboxObservedStatus::Absent,
            Some(at(9_999)),
        ));
        assert_eq!(d.action, ReconcileAction::None);
        assert!(!d.woken);
    }

    #[test]
    fn newer_message_wakes_a_soft_removed_task() {
        // RetainArchives removal + newer message → durable explicit wake to Running.
        let d = decide(&input(
            intent(
                TaskStatus::Open,
                SandboxDesiredState::Removed,
                Some(ArchiveRetention::RetainArchives),
                Some(remove_marker()),
            ),
            SandboxObservedStatus::Absent,
            Some(at(1_001)),
        ));
        assert_eq!(d.action, ReconcileAction::Start);
        assert!(d.woken);
    }

    #[test]
    fn delete_archives_removal_is_never_woken() {
        // A hard removal ignores a newer message — deletion wins.
        let d = decide(&input(
            intent(
                TaskStatus::Open,
                SandboxDesiredState::Removed,
                Some(ArchiveRetention::DeleteArchives),
                Some(remove_marker()),
            ),
            SandboxObservedStatus::Absent,
            Some(at(9_999)),
        ));
        // Not woken; the incomplete remove marker keeps remove retrying.
        assert_eq!(d.action, ReconcileAction::Remove);
        assert!(!d.woken);
    }

    // ---- Remove-marker incompleteness ----

    #[test]
    fn absent_with_incomplete_remove_marker_retries_remove() {
        let d = decide(&input(
            intent(
                TaskStatus::Open,
                SandboxDesiredState::Removed,
                Some(ArchiveRetention::DeleteArchives),
                Some(remove_marker()),
            ),
            SandboxObservedStatus::Absent,
            None,
        ));
        assert_eq!(d.action, ReconcileAction::Remove);
    }

    #[test]
    fn absent_with_cleared_remove_marker_is_converged() {
        // No marker → the archive obligation is complete, so absence is converged.
        let d = decide(&input(
            intent(TaskStatus::Open, SandboxDesiredState::Removed, None, None),
            SandboxObservedStatus::Absent,
            None,
        ));
        assert_eq!(d.action, ReconcileAction::None);
    }

    #[test]
    fn converged_stopped_with_incomplete_stop_marker_redrives_stop() {
        // Crashed between stop convergence and the completion commit: observed
        // Stopped, but the stop marker is still present → re-drive stop so the
        // owed `Stopped` fact commits and the marker clears.
        let marker = CorrelationMarker::new(OperationKind::Stop, OperationGeneration(3), at(1_000));
        let d = decide(&input(
            intent(
                TaskStatus::Open,
                SandboxDesiredState::Stopped,
                None,
                Some(marker),
            ),
            SandboxObservedStatus::Stopped,
            None,
        ));
        assert_eq!(d.action, ReconcileAction::Stop);
    }

    #[test]
    fn converged_stopped_without_a_marker_is_a_no_op() {
        // No owed commit → nothing to do.
        let d = decide(&input(
            intent(TaskStatus::Open, SandboxDesiredState::Stopped, None, None),
            SandboxObservedStatus::Stopped,
            None,
        ));
        assert_eq!(d.action, ReconcileAction::None);
    }

    #[test]
    fn a_non_remove_marker_does_not_signal_an_incomplete_removal() {
        // A stray Start-kind marker on a Removed intent is not a remove obligation.
        let marker =
            CorrelationMarker::new(OperationKind::Start, OperationGeneration(2), at(1_000));
        let d = decide(&input(
            intent(
                TaskStatus::Open,
                SandboxDesiredState::Removed,
                None,
                Some(marker),
            ),
            SandboxObservedStatus::Absent,
            None,
        ));
        assert_eq!(d.action, ReconcileAction::None);
    }

    #[test]
    fn stop_reason_on_intent_does_not_affect_the_decision() {
        // The driver reuses the persisted reason; `decide` ignores it.
        let mut i = intent(TaskStatus::Open, SandboxDesiredState::Stopped, None, None);
        i.stop_reason = Some(SandboxStopReason::IdleReaper);
        let d = decide(&input(i, SandboxObservedStatus::Ready, None));
        assert_eq!(d.action, ReconcileAction::Stop);
    }
}
