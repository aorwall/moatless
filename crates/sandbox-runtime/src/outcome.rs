//! The common result of every lifecycle operation.

use crate::operation::{IntentEffect, ObservedStatus, RuntimeEffect};

/// A non-fatal degradation the operation succeeded *through* — e.g. a PVC
/// failure that fell back to ephemeral storage. Carried on success so the UI
/// can warn without the operation being a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DegradationWarning {
    /// Stable machine code (e.g. `storage_degraded`).
    pub code: String,
    pub message: String,
}

/// Success diagnostics shared by [`super::runtime::RuntimeOutcome`] and
/// [`LifecycleOutcome`]. The **success** type never describes the operation's
/// own failure — that lives on the error types.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub degradations: Vec<DegradationWarning>,
}

impl Diagnostics {
    pub fn is_empty(&self) -> bool {
        self.degradations.is_empty()
    }
}

/// The common result of every lifecycle operation, with two independent effect
/// axes.
///
/// Independent: a stop against already-stopped compute is `Changed`+`NoOp`
/// (real, publishable intent change, no infra work); a reconciler retry of a
/// persisted remove is `Unchanged`+`Applied`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleOutcome {
    /// From the intent write. `Changed` covers any real intent change: the
    /// desired-state enum, the stop reason, or the archive retention.
    pub intent_effect: IntentEffect,
    /// From the adapter — never inferred from intent or a second status read.
    pub runtime_effect: RuntimeEffect,
    /// The status this call ended at (Ready | Starting | Stopped | Removed).
    pub observed: ObservedStatus,
    /// Non-fatal success diagnostics, republished by lifecycle.
    pub diagnostics: Diagnostics,
}

impl LifecycleOutcome {
    pub fn new(
        intent_effect: IntentEffect,
        runtime_effect: RuntimeEffect,
        observed: ObservedStatus,
    ) -> Self {
        Self {
            intent_effect,
            runtime_effect,
            observed,
            diagnostics: Diagnostics::default(),
        }
    }

    pub fn with_diagnostics(mut self, diagnostics: Diagnostics) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    /// A pure no-op: neither intent nor infrastructure changed. Reusing an
    /// already-Ready sandbox on a message-wake is the canonical case — it
    /// publishes nothing.
    pub fn is_pure_no_op(&self) -> bool {
        matches!(self.intent_effect, IntentEffect::Unchanged)
            && matches!(self.runtime_effect, RuntimeEffect::NoOp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_against_stopped_compute_is_changed_noop() {
        let outcome = LifecycleOutcome::new(
            IntentEffect::Changed,
            RuntimeEffect::NoOp,
            ObservedStatus::Stopped,
        );
        assert!(!outcome.is_pure_no_op());
        // The intent change is real and publishable even though infra did
        // nothing.
        assert!(outcome.intent_effect.changed());
        assert!(!outcome.runtime_effect.applied());
    }

    #[test]
    fn reusing_ready_sandbox_is_pure_no_op() {
        let outcome = LifecycleOutcome::new(
            IntentEffect::Unchanged,
            RuntimeEffect::NoOp,
            ObservedStatus::Ready,
        );
        assert!(outcome.is_pure_no_op());
    }

    #[test]
    fn diagnostics_round_trip() {
        let diags = Diagnostics {
            degradations: vec![DegradationWarning {
                code: "storage_degraded".into(),
                message: "ephemeral fallback".into(),
            }],
        };
        assert!(!diags.is_empty());
        let outcome = LifecycleOutcome::new(
            IntentEffect::Changed,
            RuntimeEffect::Applied,
            ObservedStatus::Starting,
        )
        .with_diagnostics(diags.clone());
        assert_eq!(outcome.diagnostics, diags);
    }
}
