//! The operation vocabulary: one operation's kind, its intended and actual
//! effect, its generation, and its correlation marker, plus the observed status
//! an operation reconciles against.
//!
//! Deliberately small, `Copy`, and side-effect free so the decision logic
//! (convergence, classification, marker acknowledgement, event derivation) can
//! be exhaustively unit-tested without any runtime, database, or event
//! transport.

use std::fmt;

use chrono::{DateTime, Utc};
use moatless_types::sandbox::SandboxStatus;

/// The five caller-facing lifecycle operations.
///
/// `is_edge` splits caller-owned edges (restart/redeploy) from level operations
/// (start/stop/remove) the reconciler recovers; see [`crate::marker`] for how
/// each clears its persisted marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationKind {
    Start,
    Stop,
    Restart,
    Redeploy,
    Remove,
}

impl OperationKind {
    /// Restart and redeploy are synchronous, caller-owned edges with no async
    /// replay: the reconciler never re-issues them.
    pub fn is_edge(self) -> bool {
        matches!(self, Self::Restart | Self::Redeploy)
    }

    /// A toward-Running operation (start / restart / redeploy). Gates the
    /// `Initializing` infrastructure event, which publishes only when such an
    /// operation actually applies a runtime effect.
    pub fn is_toward_running(self) -> bool {
        matches!(self, Self::Start | Self::Restart | Self::Redeploy)
    }

    /// Stable slug used in deterministic event ids and lock names.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Redeploy => "redeploy",
            Self::Remove => "remove",
        }
    }

    /// Parse the persisted slug back into a kind. `None` for an unknown value.
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "restart" => Some(Self::Restart),
            "redeploy" => Some(Self::Redeploy),
            "remove" => Some(Self::Remove),
            _ => None,
        }
    }
}

impl fmt::Display for OperationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Did the intent write actually change persisted state?
///
/// `Changed` covers any real change — desired-state, stop reason, or archive
/// retention — not only desired-state transitions; a stop that only rewrites
/// the reason is still `Changed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentEffect {
    Changed,
    Unchanged,
}

impl IntentEffect {
    pub fn changed(self) -> bool {
        matches!(self, Self::Changed)
    }
}

/// Did the runtime adapter actually do infrastructure work?
///
/// Must come from the adapter's [`RuntimeOutcome`]: on retries intent is
/// already persisted, so lifecycle cannot derive it from intent or an extra
/// status read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEffect {
    Applied,
    NoOp,
}

impl RuntimeEffect {
    pub fn applied(self) -> bool {
        matches!(self, Self::Applied)
    }
}

/// The status an operation reports as its immediate outcome.
///
/// Distinct from [`crate::convergence::SandboxObservedStatus`] — this is the
/// "what did this call end at" projection returned to the caller; Docker
/// reports `Ready` synchronously, Kubernetes returns `Starting` with readiness
/// arriving later via watcher observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedStatus {
    Ready,
    Starting,
    Stopped,
    Removed,
}

impl ObservedStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Starting => "starting",
            Self::Stopped => "stopped",
            Self::Removed => "removed",
        }
    }

    /// Project the operation outcome into the public Sandbox display/API state.
    pub fn sandbox_status(self) -> SandboxStatus {
        match self {
            Self::Ready => SandboxStatus::Ready,
            Self::Starting => SandboxStatus::Initializing,
            Self::Stopped => SandboxStatus::Stopped,
            Self::Removed => SandboxStatus::Removed,
        }
    }
}

impl fmt::Display for ObservedStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Monotonic, comparable operation identity stamped on the resources it
/// creates, and carried on every watcher event as the *observed* generation.
///
/// Ordering is load-bearing: observed < marker proves a superseded
/// operation. Recovery reuses the marker's generation, keeping attempt
/// accounting and event identity stable across retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationGeneration(pub i64);

impl OperationGeneration {
    pub fn value(self) -> i64 {
        self.0
    }

    /// The next generation after this one. Production allocates from a
    /// persisted per-Task counter; only the in-memory test counter advances
    /// this way.
    #[cfg(test)]
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for OperationGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The correlation marker persisted atomically alongside intent.
///
/// Outlives the adapter call — Kubernetes returns `Starting` before readiness
/// and Docker events arrive async, so clearing on return would misclassify late
/// events. Acknowledgement is the reconciler's job, per operation class (see
/// [`crate::marker`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrelationMarker {
    pub kind: OperationKind,
    pub generation: OperationGeneration,
    pub started_at: DateTime<Utc>,
}

impl CorrelationMarker {
    pub fn new(
        kind: OperationKind,
        generation: OperationGeneration,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            kind,
            generation,
            started_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_kind_edge_and_toward_running_are_disjoint_where_expected() {
        assert!(OperationKind::Restart.is_edge());
        assert!(OperationKind::Redeploy.is_edge());
        assert!(!OperationKind::Start.is_edge());
        assert!(!OperationKind::Stop.is_edge());
        assert!(!OperationKind::Remove.is_edge());

        assert!(OperationKind::Start.is_toward_running());
        assert!(OperationKind::Restart.is_toward_running());
        assert!(OperationKind::Redeploy.is_toward_running());
        assert!(!OperationKind::Stop.is_toward_running());
        assert!(!OperationKind::Remove.is_toward_running());
    }

    #[test]
    fn generation_orders_and_advances() {
        let g = OperationGeneration(7);
        assert_eq!(g.next(), OperationGeneration(8));
        assert!(OperationGeneration(3) < OperationGeneration(4));
        assert_eq!(g.to_string(), "7");
    }

    #[test]
    fn db_str_round_trips() {
        for kind in [
            OperationKind::Start,
            OperationKind::Stop,
            OperationKind::Restart,
            OperationKind::Redeploy,
            OperationKind::Remove,
        ] {
            assert_eq!(OperationKind::from_db_str(kind.as_str()), Some(kind));
        }
        assert_eq!(OperationKind::from_db_str("bogus"), None);
    }

    #[test]
    fn effect_predicates() {
        assert!(IntentEffect::Changed.changed());
        assert!(!IntentEffect::Unchanged.changed());
        assert!(RuntimeEffect::Applied.applied());
        assert!(!RuntimeEffect::NoOp.applied());
    }
}
