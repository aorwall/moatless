//! Operation-driven event derivation with deterministic identity.
//!
//! Lifecycle publishes two event classes with distinct sources. **Acceptance
//! events** derive from `IntentEffect::Changed`. **Completion events** publish
//! when an operation **newly establishes convergence**. Only
//! **infrastructure-effect events** gate on `RuntimeEffect::Applied`. Lifecycle
//! never publishes `Ready` — that is the watcher's convergence event.
//!
//! Event ids are deterministic — `sandbox:{task_id}:{operation_generation}:{fact}`
//! — and ride the existing transactional-outbox / idempotent-persistence path
//! (`ON CONFLICT (event_id) DO NOTHING`), so at-least-once retries of one
//! generation persist exactly once.

use moatless_events::{SandboxEventPayload, SandboxStatus};

use crate::task::models::SandboxStopReason;

use super::outcome::LifecycleOutcome;
use super::types::{ObservedStatus, OperationGeneration, OperationKind};

/// The deterministic fact an event records for an operation generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFact {
    /// Watcher convergence: the generation's replacement became Ready.
    Ready,
    /// Acceptance: intent moved to `Removed`.
    Removing,
    /// Completion: the operation newly established a stopped sandbox.
    Stopped,
    /// Completion: the operation newly established a removed sandbox.
    Removed,
    /// Infrastructure: start/restart/redeploy actually did work.
    Initializing,
    /// Failure fact for an attempt.
    Error,
}

impl EventFact {
    /// Stable slug used as the last segment of the deterministic event id.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Removing => "removing",
            Self::Stopped => "stopped",
            Self::Removed => "removed",
            Self::Initializing => "initializing",
            Self::Error => "error",
        }
    }

    /// The `SandboxStatus` sub-type the event publishes under.
    pub fn sandbox_status(self) -> SandboxStatus {
        match self {
            Self::Ready => SandboxStatus::Ready,
            Self::Removing => SandboxStatus::Removing,
            Self::Stopped => SandboxStatus::Stopped,
            Self::Removed => SandboxStatus::Removed,
            Self::Initializing => SandboxStatus::Initializing,
            Self::Error => SandboxStatus::Error,
        }
    }
}

/// A derived lifecycle event ready to publish through the outbox path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleEvent {
    pub event_id: String,
    pub generation: OperationGeneration,
    pub fact: EventFact,
    pub status: SandboxStatus,
    /// Carried only on the `Stopped` completion so reason tags survive
    /// (`SandboxStopReason::IdleReaper` → `idle_reaped` + initiator).
    pub stop_reason: Option<SandboxStopReason>,
}

impl LifecycleEvent {
    fn new(task_id: &str, generation: OperationGeneration, fact: EventFact, stop_reason: Option<SandboxStopReason>) -> Self {
        Self {
            event_id: event_id(task_id, generation, fact),
            generation,
            fact,
            status: fact.sandbox_status(),
            stop_reason,
        }
    }

    /// The `task_events.sub_type` value — the `SandboxStatus` snake_case slug.
    pub fn sub_type(&self) -> &'static str {
        self.status.as_str()
    }

    /// Whether this is a failure fact (persisted `is_error`).
    pub fn is_error(&self) -> bool {
        matches!(self.fact, EventFact::Error)
    }

    /// Whether the store's completion transaction persists this fact. Completion
    /// facts (`Stopped`/`Removed`) are committed to `task_events` before
    /// publication, so publication carries `payload_persisted = true`; other
    /// facts are persisted by the event consumer.
    pub fn persisted_as_completion(&self) -> bool {
        matches!(self.fact, EventFact::Stopped | EventFact::Removed)
    }

    /// The typed payload published and persisted for this event. A `Stopped`
    /// completion carries its reason tag so the UI attributes the stop.
    pub fn payload(&self) -> SandboxEventPayload {
        let mut payload = SandboxEventPayload {
            message: self.message().to_string(),
            ..Default::default()
        };
        if let Some(reason) = self.stop_reason {
            let (lifecycle_event, initiator) = stop_reason_tags(reason);
            payload.lifecycle_event = lifecycle_event.map(str::to_string);
            payload.initiator = Some(initiator.to_string());
        }
        payload
    }

    fn message(&self) -> &'static str {
        match self.fact {
            EventFact::Ready => "Sandbox ready",
            EventFact::Removing => "Removing sandbox",
            EventFact::Stopped => "Sandbox stopped",
            EventFact::Removed => "Sandbox removed",
            EventFact::Initializing => "Sandbox initializing",
            EventFact::Error => "Sandbox operation failed",
        }
    }
}

/// `(lifecycle_event, initiator)` tags carried on a `Stopped` completion so the
/// UI attributes an intentional stop (idle stop → `idle_reaped`/`reaper`, crash →
/// `crash_autostop`), matching the tags the reaper and pod watcher stamp.
fn stop_reason_tags(reason: SandboxStopReason) -> (Option<&'static str>, &'static str) {
    match reason {
        SandboxStopReason::User => (None, "user"),
        SandboxStopReason::IdleReaper => (Some("idle_reaped"), "reaper"),
        SandboxStopReason::Cleanup => (None, "close"),
        SandboxStopReason::System => (None, "system"),
        SandboxStopReason::Error => (Some("crash_autostop"), "crash_autostop"),
    }
}

/// The deterministic event id for a `(task, generation, fact)` triple.
pub fn event_id(task_id: &str, generation: OperationGeneration, fact: EventFact) -> String {
    format!("sandbox:{task_id}:{generation}:{}", fact.slug())
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
    /// this attempt? Load-bearing for recovery: a retry that is
    /// `Unchanged` + `NoOp` yet completes an incomplete remove still newly
    /// establishes convergence.
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
    /// target **and** any of intent-changed, runtime-applied, or an incomplete
    /// marker proves it (not a pre-effect status read) that the operation did
    /// the establishing.
    fn newly_establishes(&self, target: ObservedStatus) -> bool {
        self.observed == target
            && (self.intent_changed || self.runtime_applied || self.marker_was_incomplete)
    }
}

/// Acceptance events — emitted right after the atomic intent write, **before**
/// the runtime effect, so a later runtime failure still records that intent
/// moved. Currently only `Removing`.
pub fn acceptance_events(task_id: &str, ctx: &EventContext) -> Vec<LifecycleEvent> {
    let mut out = Vec::new();
    if ctx.op == OperationKind::Remove && ctx.intent_changed {
        out.push(LifecycleEvent::new(task_id, ctx.generation, EventFact::Removing, None));
    }
    out
}

/// Completion + infrastructure events — emitted after a successful runtime
/// effect.
///
/// - Completion (`Stopped` / `Removed`) publishes when the operation newly
///   establishes convergence — including `NoOp` completions (stopping
///   already-stopped compute is a real stop) and recovery retries.
/// - Infrastructure (`Initializing`) gates on `RuntimeEffect::Applied`.
/// - A pure `Unchanged` + `NoOp` against an already-converged target publishes
///   nothing.
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
                out.push(LifecycleEvent::new(task_id, ctx.generation, EventFact::Removed, None));
            }
        }
        OperationKind::Start | OperationKind::Restart | OperationKind::Redeploy => {}
    }

    // Infrastructure-effect event — only when the adapter did work.
    if ctx.op.is_toward_running() && ctx.runtime_applied {
        out.push(LifecycleEvent::new(task_id, ctx.generation, EventFact::Initializing, None));
    }

    out
}

/// The deterministic `Error` fact for a failed attempt of this generation.
/// One per operation generation — at-least-once retries of the same generation
/// dedup on the deterministic id.
pub fn error_event(task_id: &str, generation: OperationGeneration) -> LifecycleEvent {
    LifecycleEvent::new(task_id, generation, EventFact::Error, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::lifecycle::outcome::LifecycleOutcome;
    use crate::sandbox::lifecycle::types::{IntentEffect, RuntimeEffect};

    fn gen(n: i64) -> OperationGeneration {
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
            generation: gen(5),
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
        assert_eq!(event_id("task-1", gen(5), EventFact::Removed), "sandbox:task-1:5:removed");
        assert_eq!(event_id("task-1", gen(6), EventFact::Removed), "sandbox:task-1:6:removed");
        assert_eq!(event_id("task-1", gen(5), EventFact::Error), "sandbox:task-1:5:error");
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
        // outbox's ON CONFLICT dedups them to one persisted event.
        let a = outcome_events("t", &ctx(OperationKind::Remove, true, true, ObservedStatus::Removed, false));
        let b = outcome_events("t", &ctx(OperationKind::Remove, false, true, ObservedStatus::Removed, false));
        assert_eq!(a[0].event_id, b[0].event_id);
    }

    // ---- Acceptance ----

    #[test]
    fn removing_acceptance_fires_only_when_intent_moves() {
        let changed = acceptance_events("t", &ctx(OperationKind::Remove, true, false, ObservedStatus::Removed, false));
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].fact, EventFact::Removing);
        assert_eq!(changed[0].status, SandboxStatus::Removing);

        // A retry (intent unchanged) emits no fresh acceptance event.
        let unchanged = acceptance_events("t", &ctx(OperationKind::Remove, false, true, ObservedStatus::Removed, false));
        assert!(unchanged.is_empty());
    }

    #[test]
    fn non_remove_ops_emit_no_acceptance() {
        for op in [OperationKind::Start, OperationKind::Stop, OperationKind::Restart, OperationKind::Redeploy] {
            assert!(acceptance_events("t", &ctx(op, true, true, ObservedStatus::Ready, false)).is_empty());
        }
    }

    // ---- Completion (stop) ----

    #[test]
    fn stopping_already_stopped_compute_still_publishes_stopped() {
        // Changed intent + NoOp runtime + observed Stopped → a real stop.
        let events = outcome_events("t", &ctx(OperationKind::Stop, true, false, ObservedStatus::Stopped, false));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fact, EventFact::Stopped);
    }

    #[test]
    fn stop_carries_reason_tag() {
        let mut c = ctx(OperationKind::Stop, true, true, ObservedStatus::Stopped, false);
        c.stop_reason = Some(SandboxStopReason::IdleReaper);
        let events = outcome_events("t", &c);
        assert_eq!(events[0].stop_reason, Some(SandboxStopReason::IdleReaper));
    }

    // ---- Completion (remove) + recovery ----

    #[test]
    fn recovery_retry_unchanged_applied_publishes_removed() {
        // First attempt persisted Removed and failed (Error). The reconciler's
        // retry is Unchanged + Applied yet newly establishes convergence.
        let events = outcome_events("t", &ctx(OperationKind::Remove, false, true, ObservedStatus::Removed, false));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fact, EventFact::Removed);
    }

    #[test]
    fn recovery_retry_unchanged_noop_with_incomplete_marker_publishes_removed() {
        // Even a fully idempotent retry (Unchanged + NoOp) publishes Removed
        // when an incomplete marker proves the obligation was outstanding.
        let events = outcome_events("t", &ctx(OperationKind::Remove, false, false, ObservedStatus::Removed, true));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fact, EventFact::Removed);
    }

    // ---- No-op suppression ----

    #[test]
    fn pure_unchanged_noop_publishes_nothing() {
        // Reusing an already-Ready sandbox on a message-wake.
        let events = outcome_events("t", &ctx(OperationKind::Start, false, false, ObservedStatus::Ready, false));
        assert!(events.is_empty());
        // And a remove no-op against an already-converged (no marker) target.
        let events = outcome_events("t", &ctx(OperationKind::Remove, false, false, ObservedStatus::Removed, false));
        assert!(events.is_empty());
    }

    // ---- Infrastructure ----

    #[test]
    fn initializing_gates_on_runtime_applied() {
        // start that did work → Initializing.
        let events = outcome_events("t", &ctx(OperationKind::Start, true, true, ObservedStatus::Starting, false));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fact, EventFact::Initializing);

        // start that reused Ready (NoOp) → nothing.
        let events = outcome_events("t", &ctx(OperationKind::Start, false, false, ObservedStatus::Ready, false));
        assert!(events.is_empty());
    }

    #[test]
    fn restart_that_applies_publishes_initializing_not_ready() {
        let events = outcome_events("t", &ctx(OperationKind::Restart, false, true, ObservedStatus::Starting, false));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fact, EventFact::Initializing);
        // Lifecycle never publishes Ready.
        assert!(events.iter().all(|e| e.status != SandboxStatus::Ready));
    }

    // ---- Error ----

    #[test]
    fn error_fact_is_generation_scoped() {
        let e = error_event("t", gen(5));
        assert_eq!(e.fact, EventFact::Error);
        assert_eq!(e.event_id, "sandbox:t:5:error");
    }

    // ---- Full remove lifecycle: acceptance + completion coexist ----

    #[test]
    fn first_successful_remove_emits_removing_then_removed() {
        let c = ctx(OperationKind::Remove, true, true, ObservedStatus::Removed, false);
        let acc = acceptance_events("t", &c);
        let out = outcome_events("t", &c);
        assert_eq!(acc[0].fact, EventFact::Removing);
        assert_eq!(out[0].fact, EventFact::Removed);
        // Distinct ids — both persist.
        assert_ne!(acc[0].event_id, out[0].event_id);
    }

    // ---- Publish/persist translation ----

    fn built(fact: EventFact, reason: Option<SandboxStopReason>) -> LifecycleEvent {
        LifecycleEvent::new("t", gen(1), fact, reason)
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
    fn completion_facts_are_marked_persisted_others_are_not() {
        assert!(built(EventFact::Stopped, None).persisted_as_completion());
        assert!(built(EventFact::Removed, None).persisted_as_completion());
        assert!(!built(EventFact::Removing, None).persisted_as_completion());
        assert!(!built(EventFact::Initializing, None).persisted_as_completion());
        assert!(!built(EventFact::Error, None).persisted_as_completion());
        // sub_type mirrors the SandboxStatus slug; Error is the only is_error fact.
        assert_eq!(built(EventFact::Removed, None).sub_type(), "removed");
        assert!(built(EventFact::Error, None).is_error());
        assert!(!built(EventFact::Stopped, None).is_error());
    }

    #[test]
    fn from_outcome_maps_effect_axes() {
        let outcome = LifecycleOutcome::new(IntentEffect::Changed, RuntimeEffect::NoOp, ObservedStatus::Stopped);
        let c = EventContext::from_outcome(OperationKind::Stop, gen(9), &outcome, false, Some(SandboxStopReason::User));
        assert!(c.intent_changed);
        assert!(!c.runtime_applied);
        let events = outcome_events("t", &c);
        assert_eq!(events[0].fact, EventFact::Stopped);
        assert_eq!(events[0].stop_reason, Some(SandboxStopReason::User));
    }
}
