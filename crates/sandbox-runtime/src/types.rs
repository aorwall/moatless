//! Core lifecycle vocabulary.
//!
//! These types are the shared alphabet the rest of the module reasons over.
//! They are deliberately small, `Copy`, and side-effect free so the decision
//! logic (convergence, classification, marker acknowledgement, event
//! derivation) can be exhaustively unit-tested without any runtime, database,
//! or event transport.

use std::fmt;

use chrono::{DateTime, Utc};
use moatless_types::sandbox::SandboxStatus;

/// The five caller-facing lifecycle operations.
///
/// `is_edge` distinguishes the caller-owned edges (restart/redeploy) from the
/// level operations (start/stop/remove) that the reconciler recovers. The
/// spec keys several rules on this split — edge markers clear only when the
/// *replacement* generation is observed Ready, whereas level markers clear on
/// observed convergence plus a durable completion fact.
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
/// `Changed` covers **any** real intent change — the desired-state enum, the
/// stop reason, or the archive retention — not only desired-state transitions.
/// A stop that only rewrites the reason is still `Changed`.
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
/// The effect **must** come from the adapter's [`RuntimeOutcome`]. On retries
/// intent is already persisted, so lifecycle cannot derive "did anything
/// change" from intent, and it must not infer it through an extra status read.
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
/// Distinct from [`crate::convergence::SandboxObservedStatus`]:
/// this is the small "what did this call end at" projection returned to the
/// caller. Adapters legitimately differ — Docker completes transitions
/// synchronously and reports `Ready`; Kubernetes returns `Starting` and
/// readiness arrives later through watcher observation.
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

/// `ArchiveRetention` lives in `task::models` (tasks band): the close, reap, and
/// delete sites that choose a retention policy are tasks-band and name it
/// intra-band, while the lifecycle store, providers, and reconciler reach it on
/// the legal downward edge. Re-exported here so `super::types::ArchiveRetention`
/// and the `lifecycle` module's re-export keep resolving unchanged.
pub use moatless_types::sandbox::ArchiveRetention;

/// Monotonic, comparable operation identity.
///
/// Stamped onto the runtime resources an operation creates (K8s pod-template
/// label, Docker container label) and carried on every watcher event as the
/// *observed* generation. Ordering is load-bearing: an observed generation
/// **less than** the marker generation proves the event belongs to a
/// superseded operation (an old pod's deliberate death during restart), which
/// the watcher suppresses.
///
/// A newly requested operation allocates the next generation; recovery of an
/// incomplete operation **reuses** its marker generation rather than minting a
/// new one, so attempt accounting and event identity stay stable across
/// retries. "generation" is qualified as *operation* generation throughout to
/// avoid colliding with Kubernetes resource generations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationGeneration(pub i64);

impl OperationGeneration {
    pub fn value(self) -> i64 {
        self.0
    }

    /// The next generation after this one. Production allocates from a persisted
    /// per-Task counter; only the in-memory test counter advances this way.
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
/// The marker outlives the adapter call — Kubernetes returns `Starting` before
/// readiness and Docker events arrive asynchronously, so clearing on return
/// would misclassify late events. Acknowledgement is the reconciler's job and
/// is per operation class (see [`crate::marker`]).
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
    fn archive_retention_semantics() {
        assert!(ArchiveRetention::RetainArchives.retains());
        assert!(!ArchiveRetention::DeleteArchives.retains());
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

        for r in [
            ArchiveRetention::RetainArchives,
            ArchiveRetention::DeleteArchives,
        ] {
            assert_eq!(ArchiveRetention::from_db_str(r.as_str()), Some(r));
        }
        assert_eq!(ArchiveRetention::from_db_str("bogus"), None);
    }

    #[test]
    fn effect_predicates() {
        assert!(IntentEffect::Changed.changed());
        assert!(!IntentEffect::Unchanged.changed());
        assert!(RuntimeEffect::Applied.applied());
        assert!(!RuntimeEffect::NoOp.applied());
    }
}
