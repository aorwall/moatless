//! Operation-driven event derivation with deterministic identity.
//!
//! [`acceptance_events`] and [`outcome_events`] derive Acceptance, Completion,
//! and Infrastructure-effect facts respectively. Lifecycle never publishes
//! `Ready` — that is the watcher's convergence event.
//!
//! Event ids (`sandbox:{task_id}:{operation_generation}:{fact}`) ride `ON
//! CONFLICT (event_id) DO NOTHING`, so retries persist exactly once.

use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus};

use moatless_types::sandbox::SandboxStopReason;

use moatless_sandbox_runtime::observation_events::{EventFact, event_id};
use moatless_sandbox_runtime::operation::{ObservedStatus, OperationGeneration, OperationKind};
use moatless_sandbox_runtime::outcome::LifecycleOutcome;

/// Why an operation failed, carried on a failure fact so the durable event
/// explains itself instead of reading "Sandbox operation failed".
///
/// Adapters surface everything through
/// [`RuntimeError`](moatless_sandbox_runtime::runtime::RuntimeError); this is
/// where lifecycle republishes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureDetail {
    /// The operation that failed. Named in the message, since the coarse
    /// `error` sub-type alone never said whether a start, a stop, or a remove
    /// was in flight.
    pub op: OperationKind,
    /// Bounded cause slug published as `lifecycle_event` — a UI label key, so
    /// it must stay a small closed vocabulary.
    pub cause: &'static str,
    /// The full error chain, published as `error`.
    pub reason: String,
}

impl FailureDetail {
    pub fn new(op: OperationKind, cause: &'static str, reason: impl Into<String>) -> Self {
        Self {
            op,
            cause,
            reason: reason.into(),
        }
    }
}

/// A derived lifecycle event ready to persist and publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleEvent {
    pub event_id: String,
    pub generation: OperationGeneration,
    pub fact: EventFact,
    pub status: SandboxStatus,
    /// Carried only on the `Stopped` completion so reason tags survive
    /// (`SandboxStopReason::IdleReaper` → `idle_reaped` + initiator).
    pub stop_reason: Option<SandboxStopReason>,
    /// Carried on the failure facts (`Error`, `RecoveryExhausted`) so the
    /// published payload names the operation and its cause.
    pub failure: Option<FailureDetail>,
}

impl LifecycleEvent {
    fn new(
        task_id: &str,
        generation: OperationGeneration,
        fact: EventFact,
        stop_reason: Option<SandboxStopReason>,
    ) -> Self {
        Self {
            event_id: event_id(task_id, generation, fact),
            generation,
            fact,
            status: fact.sandbox_status(),
            stop_reason,
            failure: None,
        }
    }

    /// The `task_events.sub_type` value — the `SandboxStatus` `snake_case`
    /// slug.
    pub fn sub_type(&self) -> &'static str {
        self.status.as_str()
    }

    /// Whether this is a failure fact (persisted `is_error`).
    pub fn is_error(&self) -> bool {
        matches!(self.fact, EventFact::Error | EventFact::RecoveryExhausted)
    }

    /// The typed payload published and persisted for this event.
    ///
    /// A `Stopped` completion carries its reason tag; a failure fact carries
    /// the operation, cause slug, and full error chain, so the event feed,
    /// transcript, and Linear activity each read a real reason.
    pub fn payload(&self) -> SandboxEventPayload {
        let mut payload = SandboxEventPayload {
            message: self.message(),
            ..Default::default()
        };
        if let Some(reason) = self.stop_reason {
            let (lifecycle_event, initiator) = stop_reason_tags(reason);
            payload.lifecycle_event = lifecycle_event.map(str::to_string);
            payload.initiator = Some(initiator.to_string());
        }
        if let Some(failure) = &self.failure {
            payload.lifecycle_event = Some(failure.cause.to_string());
            payload.error = Some(failure.reason.clone());
        }
        payload
    }

    fn message(&self) -> String {
        let generic = match self.fact {
            EventFact::Ready => "Sandbox ready",
            EventFact::Removing => "Removing sandbox",
            EventFact::Stopped => "Sandbox stopped",
            EventFact::Removed => "Sandbox removed",
            EventFact::Initializing => "Sandbox initializing",
            EventFact::Error => "Sandbox operation failed",
            EventFact::RecoveryExhausted => "Sandbox recovery exhausted",
        };
        match &self.failure {
            Some(failure) if self.fact == EventFact::RecoveryExhausted => format!(
                "Gave up recovering sandbox {} after repeated failures: {}",
                failure.op, failure.reason
            ),
            Some(failure) => format!("Sandbox {} failed: {}", failure.op, failure.reason),
            None => generic.to_string(),
        }
    }
}

/// `(lifecycle_event, initiator)` tags carried on a `Stopped` completion so the
/// UI attributes an intentional stop (idle stop → `idle_reaped`/`reaper`, crash
/// → `crash_autostop`), matching the tags the reaper and pod watcher stamp.
fn stop_reason_tags(reason: SandboxStopReason) -> (Option<&'static str>, &'static str) {
    match reason {
        SandboxStopReason::User => (None, "user"),
        SandboxStopReason::IdleReaper => (Some("idle_reaped"), "reaper"),
        SandboxStopReason::Cleanup => (None, "close"),
        SandboxStopReason::System => (None, "system"),
        SandboxStopReason::Error => (Some("crash_autostop"), "crash_autostop"),
    }
}

/// Everything the derivation needs about a completed operation.
#[derive(Debug, Clone, Copy)]
pub struct EventContext {
    pub op: OperationKind,
    pub generation: OperationGeneration,
    /// Did intent actually change?
    pub intent_changed: bool,
    /// Did the adapter do infrastructure work?
    pub runtime_applied: bool,
    /// The status the operation ended at.
    pub observed: ObservedStatus,
    /// Was a marker for this operation already persisted-but-incomplete before
    /// this attempt?
    ///
    /// Load-bearing for recovery: an `Unchanged` + `NoOp` retry that completes
    /// an incomplete remove still newly establishes convergence.
    pub marker_was_incomplete: bool,
    /// The stop reason to tag onto a `Stopped` completion.
    pub stop_reason: Option<SandboxStopReason>,
}

impl EventContext {
    pub fn from_outcome(
        op: OperationKind,
        generation: OperationGeneration,
        outcome: &LifecycleOutcome,
        marker_was_incomplete: bool,
        stop_reason: Option<SandboxStopReason>,
    ) -> Self {
        Self {
            op,
            generation,
            intent_changed: outcome.intent_effect.changed(),
            runtime_applied: outcome.runtime_effect.applied(),
            observed: outcome.observed,
            marker_was_incomplete,
            stop_reason,
        }
    }

    /// An operation newly establishes convergence when it ends converged on its
    /// target.
    ///
    /// **And** intent-changed, runtime-applied, or an incomplete marker proves
    /// this attempt (not a pre-effect status read) did the establishing.
    fn newly_establishes(&self, target: ObservedStatus) -> bool {
        self.observed == target
            && (self.intent_changed || self.runtime_applied || self.marker_was_incomplete)
    }
}

/// Acceptance events derive from `IntentEffect::Changed`, emitted right after
/// the atomic intent write and **before** the runtime effect, so a later
/// runtime failure still records that intent moved. Currently only
/// `Removing`.
pub fn acceptance_events(task_id: &str, ctx: &EventContext) -> Vec<LifecycleEvent> {
    let mut out = Vec::new();
    if ctx.op == OperationKind::Remove && ctx.intent_changed {
        out.push(LifecycleEvent::new(
            task_id,
            ctx.generation,
            EventFact::Removing,
            None,
        ));
    }
    out
}

/// Completion and infrastructure events after a successful runtime effect.
///
/// Completion (`Stopped`/`Removed`) publishes on newly-established
/// convergence, including `NoOp` completions and retries; infrastructure
/// (`Initializing`) gates on `RuntimeEffect::Applied`, and a pure
/// `Unchanged`+`NoOp` publishes nothing.
pub fn outcome_events(task_id: &str, ctx: &EventContext) -> Vec<LifecycleEvent> {
    let mut out = Vec::new();

    match ctx.op {
        OperationKind::Stop => {
            if ctx.newly_establishes(ObservedStatus::Stopped) {
                out.push(LifecycleEvent::new(
                    task_id,
                    ctx.generation,
                    EventFact::Stopped,
                    ctx.stop_reason,
                ));
            }
        }
        OperationKind::Remove => {
            if ctx.newly_establishes(ObservedStatus::Removed) {
                out.push(LifecycleEvent::new(
                    task_id,
                    ctx.generation,
                    EventFact::Removed,
                    None,
                ));
            }
        }
        OperationKind::Start | OperationKind::Restart | OperationKind::Redeploy => {}
    }

    // Infrastructure-effect event — only when the adapter did work.
    if ctx.op.is_toward_running() && ctx.runtime_applied {
        out.push(LifecycleEvent::new(
            task_id,
            ctx.generation,
            EventFact::Initializing,
            None,
        ));
    }

    out
}

/// The deterministic `Error` fact for a failed attempt of this generation.
///
/// One per generation — retries dedup on the id, so it records the **first**
/// failure; exhausting the reconciler's cap instead publishes
/// [`recovery_exhausted_event`].
pub fn error_event(
    task_id: &str,
    generation: OperationGeneration,
    failure: FailureDetail,
) -> LifecycleEvent {
    LifecycleEvent {
        failure: Some(failure),
        ..LifecycleEvent::new(task_id, generation, EventFact::Error, None)
    }
}

/// The deterministic terminal fact for a generation whose reconciler attempt
/// cap is spent. Its own slug (not `error`) so it survives the `Error` fact's
/// dedup and the give-up is visible rather than inferred from silence.
pub fn recovery_exhausted_event(
    task_id: &str,
    generation: OperationGeneration,
    failure: FailureDetail,
) -> LifecycleEvent {
    LifecycleEvent {
        failure: Some(failure),
        ..LifecycleEvent::new(task_id, generation, EventFact::RecoveryExhausted, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_sandbox_runtime::observation_events::{EventFact, event_id};
    use moatless_sandbox_runtime::operation::{IntentEffect, RuntimeEffect};
    use moatless_sandbox_runtime::outcome::LifecycleOutcome;

    fn op_gen(n: i64) -> OperationGeneration {
        OperationGeneration(n)
    }

    fn ctx(
        op: OperationKind,
        intent_changed: bool,
        runtime_applied: bool,
        observed: ObservedStatus,
        marker_was_incomplete: bool,
    ) -> EventContext {
        EventContext {
            op,
            generation: op_gen(5),
            intent_changed,
            runtime_applied,
            observed,
            marker_was_incomplete,
            stop_reason: None,
        }
    }

    // ---- Deterministic identity ----

    #[test]
    fn event_id_is_deterministic_and_generation_scoped() {
        assert_eq!(
            event_id("task-1", op_gen(5), EventFact::Removed),
            "sandbox:task-1:5:removed"
        );
        assert_eq!(
            event_id("task-1", op_gen(6), EventFact::Removed),
            "sandbox:task-1:6:removed"
        );
        assert_eq!(
            event_id("task-1", op_gen(5), EventFact::Error),
            "sandbox:task-1:5:error"
        );
    }

    #[test]
    fn watcher_ready_convergence_id_is_generation_scoped() {
        assert_eq!(
            event_id("task-7", OperationGeneration(12), EventFact::Ready),
            "sandbox:task-7:12:ready"
        );
    }

    #[test]
    fn retries_of_one_generation_share_the_completion_id() {
        // Two retries of the same generation produce the same Removed id, so the
        // event row's ON CONFLICT dedups them to one persisted event.
        let a = outcome_events(
            "t",
            &ctx(
                OperationKind::Remove,
                true,
                true,
                ObservedStatus::Removed,
                false,
            ),
        );
        let b = outcome_events(
            "t",
            &ctx(
                OperationKind::Remove,
                false,
                true,
                ObservedStatus::Removed,
                false,
            ),
        );
        assert_eq!(a[0].event_id, b[0].event_id);
    }

    // ---- Acceptance ----

    #[test]
    fn removing_acceptance_fires_only_when_intent_moves() {
        let changed = acceptance_events(
            "t",
            &ctx(
                OperationKind::Remove,
                true,
                false,
                ObservedStatus::Removed,
                false,
            ),
        );
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].fact, EventFact::Removing);
        assert_eq!(changed[0].status, SandboxStatus::Removing);

        // A retry (intent unchanged) emits no fresh acceptance event.
        let unchanged = acceptance_events(
            "t",
            &ctx(
                OperationKind::Remove,
                false,
                true,
                ObservedStatus::Removed,
                false,
            ),
        );
        assert!(unchanged.is_empty());
    }

    #[test]
    fn non_remove_ops_emit_no_acceptance() {
        for op in [
            OperationKind::Start,
            OperationKind::Stop,
            OperationKind::Restart,
            OperationKind::Redeploy,
        ] {
            assert!(
                acceptance_events("t", &ctx(op, true, true, ObservedStatus::Ready, false))
                    .is_empty()
            );
        }
    }

    // ---- Completion (stop) ----

    #[test]
    fn stopping_already_stopped_compute_still_publishes_stopped() {
        // Changed intent + NoOp runtime + observed Stopped → a real stop.
        let events = outcome_events(
            "t",
            &ctx(
                OperationKind::Stop,
                true,
                false,
                ObservedStatus::Stopped,
                false,
            ),
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fact, EventFact::Stopped);
    }

    #[test]
    fn stop_carries_reason_tag() {
        let mut c = ctx(
            OperationKind::Stop,
            true,
            true,
            ObservedStatus::Stopped,
            false,
        );
        c.stop_reason = Some(SandboxStopReason::IdleReaper);
        let events = outcome_events("t", &c);
        assert_eq!(events[0].stop_reason, Some(SandboxStopReason::IdleReaper));
    }

    // ---- Completion (remove) + recovery ----

    #[test]
    fn recovery_retry_unchanged_applied_publishes_removed() {
        // First attempt persisted Removed and failed (Error). The reconciler's
        // retry is Unchanged + Applied yet newly establishes convergence.
        let events = outcome_events(
            "t",
            &ctx(
                OperationKind::Remove,
                false,
                true,
                ObservedStatus::Removed,
                false,
            ),
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fact, EventFact::Removed);
    }

    #[test]
    fn recovery_retry_unchanged_noop_with_incomplete_marker_publishes_removed() {
        // Even a fully idempotent retry (Unchanged + NoOp) publishes Removed
        // when an incomplete marker proves the obligation was outstanding.
        let events = outcome_events(
            "t",
            &ctx(
                OperationKind::Remove,
                false,
                false,
                ObservedStatus::Removed,
                true,
            ),
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fact, EventFact::Removed);
    }

    // ---- No-op suppression ----

    #[test]
    fn pure_unchanged_noop_publishes_nothing() {
        // Reusing an already-Ready sandbox on a message-wake.
        let events = outcome_events(
            "t",
            &ctx(
                OperationKind::Start,
                false,
                false,
                ObservedStatus::Ready,
                false,
            ),
        );
        assert!(events.is_empty());
        // And a remove no-op against an already-converged (no marker) target.
        let events = outcome_events(
            "t",
            &ctx(
                OperationKind::Remove,
                false,
                false,
                ObservedStatus::Removed,
                false,
            ),
        );
        assert!(events.is_empty());
    }

    // ---- Infrastructure ----

    #[test]
    fn initializing_gates_on_runtime_applied() {
        // start that did work → Initializing.
        let events = outcome_events(
            "t",
            &ctx(
                OperationKind::Start,
                true,
                true,
                ObservedStatus::Starting,
                false,
            ),
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fact, EventFact::Initializing);

        // start that reused Ready (NoOp) → nothing.
        let events = outcome_events(
            "t",
            &ctx(
                OperationKind::Start,
                false,
                false,
                ObservedStatus::Ready,
                false,
            ),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn restart_that_applies_publishes_initializing_not_ready() {
        let events = outcome_events(
            "t",
            &ctx(
                OperationKind::Restart,
                false,
                true,
                ObservedStatus::Starting,
                false,
            ),
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fact, EventFact::Initializing);
        // Lifecycle never publishes Ready.
        assert!(events.iter().all(|e| e.status != SandboxStatus::Ready));
    }

    // ---- Error ----

    fn timeout_failure() -> FailureDetail {
        FailureDetail::new(
            OperationKind::Start,
            "runtime_infrastructure",
            "runtime infrastructure error: Timed out after 90s waiting for the previous \
             sandbox pod to terminate",
        )
    }

    #[test]
    fn error_fact_is_generation_scoped() {
        let e = error_event("t", op_gen(5), timeout_failure());
        assert_eq!(e.fact, EventFact::Error);
        assert_eq!(e.event_id, "sandbox:t:5:error");
    }

    #[test]
    fn error_payload_names_the_operation_cause_and_reason() {
        let payload = error_event("t", op_gen(5), timeout_failure()).payload();
        // The feed and the Linear activity read `message`; it must name the
        // operation and cause.
        assert!(
            payload.message.starts_with("Sandbox start failed: "),
            "unexpected message: {}",
            payload.message
        );
        assert!(payload.message.contains("Timed out after 90s"));
        // The conversation transcript reads `error`; absent it renders
        // "Unknown agent error".
        assert!(
            payload
                .error
                .as_deref()
                .is_some_and(|e| e.contains("Timed out after 90s"))
        );
        // The UI label vocabulary reads `lifecycle_event`.
        assert_eq!(
            payload.lifecycle_event.as_deref(),
            Some("runtime_infrastructure")
        );
    }

    #[test]
    fn recovery_exhausted_does_not_collide_with_the_error_fact() {
        let failure = FailureDetail::new(OperationKind::Start, "recovery_exhausted", "5 attempts");
        let exhausted = recovery_exhausted_event("t", op_gen(5), failure);
        assert_eq!(exhausted.event_id, "sandbox:t:5:recovery_exhausted");
        assert_ne!(
            exhausted.event_id,
            error_event("t", op_gen(5), timeout_failure()).event_id
        );
        // Same sub_type as Error so existing error consumers surface it.
        assert_eq!(exhausted.sub_type(), "error");
        assert!(exhausted.is_error());
        assert!(
            exhausted
                .payload()
                .message
                .starts_with("Gave up recovering")
        );
    }

    // ---- Full remove lifecycle: acceptance + completion coexist ----

    #[test]
    fn first_successful_remove_emits_removing_then_removed() {
        let c = ctx(
            OperationKind::Remove,
            true,
            true,
            ObservedStatus::Removed,
            false,
        );
        let acc = acceptance_events("t", &c);
        let out = outcome_events("t", &c);
        assert_eq!(acc[0].fact, EventFact::Removing);
        assert_eq!(out[0].fact, EventFact::Removed);
        // Distinct ids — both persist.
        assert_ne!(acc[0].event_id, out[0].event_id);
    }

    // ---- Publish/persist translation ----

    fn built(fact: EventFact, reason: Option<SandboxStopReason>) -> LifecycleEvent {
        LifecycleEvent::new("t", op_gen(1), fact, reason)
    }

    #[test]
    fn stopped_payload_carries_reason_tags() {
        let idle = built(EventFact::Stopped, Some(SandboxStopReason::IdleReaper)).payload();
        assert_eq!(idle.lifecycle_event.as_deref(), Some("idle_reaped"));
        assert_eq!(idle.initiator.as_deref(), Some("reaper"));

        let user = built(EventFact::Stopped, Some(SandboxStopReason::User)).payload();
        assert_eq!(user.lifecycle_event, None);
        assert_eq!(user.initiator.as_deref(), Some("user"));
    }

    #[test]
    fn only_the_failure_facts_are_errors() {
        // sub_type mirrors the SandboxStatus slug.
        assert_eq!(built(EventFact::Removed, None).sub_type(), "removed");
        assert!(built(EventFact::Error, None).is_error());
        assert!(built(EventFact::RecoveryExhausted, None).is_error());
        assert!(!built(EventFact::Stopped, None).is_error());
    }

    #[test]
    fn from_outcome_maps_effect_axes() {
        let outcome = LifecycleOutcome::new(
            IntentEffect::Changed,
            RuntimeEffect::NoOp,
            ObservedStatus::Stopped,
        );
        let c = EventContext::from_outcome(
            OperationKind::Stop,
            op_gen(9),
            &outcome,
            false,
            Some(SandboxStopReason::User),
        );
        assert!(c.intent_changed);
        assert!(!c.runtime_applied);
        let events = outcome_events("t", &c);
        assert_eq!(events[0].fact, EventFact::Stopped);
        assert_eq!(events[0].stop_reason, Some(SandboxStopReason::User));
    }
}
