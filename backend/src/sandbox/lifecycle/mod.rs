//! Durable lifecycle intent and orchestration for a Task's Sandbox.
//!
//! A deep module: callers identify a Task and request a lifecycle transition;
//! the module resolves intent, persists it before any runtime effect,
//! serializes per Task, invokes the runtime adapter behind the
//! [`SandboxRuntime`] seam, records bookkeeping, publishes operation-level
//! events, and returns a typed [`LifecycleOutcome`]. Callers never supply a
//! provision request, repositories, an event publisher, or the runtime adapter
//! — the curated re-exports below are the whole surface, and the decision and
//! implementation submodules are `pub(crate)` so a lifecycle mutation cannot
//! bypass [`Lifecycle`].
//!
//! Guarantees:
//!
//! - **Intent before effect.** Desired state, stop reason, retention, and the
//!   correlation marker are written atomically before the runtime effect; a
//!   runtime failure leaves the requested target for recovery.
//! - **Deterministic, deduplicated events.** Operation events use the id
//!   `sandbox:{task}:{operation_generation}:{fact}` over the idempotent outbox,
//!   so at-least-once retries of one generation persist once. Completion facts
//!   (`Stopped`/`Removed`) are persisted and the marker cleared in one
//!   transaction before publication.
//! - **Strict removal.** A remove reports success only on observed absence of
//!   every resource (PVC included) and durable persistence of its completion
//!   fact; the remove marker survives — never age-cleared — until then.
//! - **Correlation.** The operation generation is threaded to every runtime
//!   mutation and carried on watcher events, so superseded observations are
//!   distinguished by identity, not the clock.
//!
//! The decision logic lives in [`convergence`] (reconciler desired/observed
//! map), [`classify`] (watcher four-case classification), [`marker`] (per-class
//! acknowledgement), [`wake`] (message-wake causal precedence), [`reconcile`]
//! (per-Task reconcile decision combining wake and convergence), [`events`]
//! (event derivation), and [`attempt`] (attempt cap + backoff).

// The pure decision core. Its in-crate consumers — the desired-state
// reconciler and the pod/container watchers — arrive with the caller migration;
// until then these tested functions have no non-test caller, so dead_code is
// allowed deliberately rather than by widening their visibility to `pub`.
// reason: staged delivery — decision logic lands before the code that drives it.
#[allow(dead_code)]
pub(crate) mod attempt;
#[allow(dead_code)]
pub(crate) mod classify;
#[allow(dead_code)]
pub(crate) mod convergence;
#[allow(dead_code)]
pub(crate) mod marker;
#[allow(dead_code)]
pub(crate) mod reconcile;
#[allow(dead_code)]
pub(crate) mod reconciler;
#[allow(dead_code)]
pub(crate) mod wake;

pub(crate) mod error;
pub(crate) mod events;
pub(crate) mod orchestrator;
pub(crate) mod outcome;
pub(crate) mod runtime;
pub(crate) mod types;

// Infrastructure adapters implementing the ports, `pub(crate)` and constructed
// by `AppState` when it builds the `Lifecycle` handle.
pub(crate) mod agent_stop;
pub(crate) mod dao;
pub(crate) mod event_sink;
pub(crate) mod lock;
pub(crate) mod runtime_bridge;

// The lifecycle contract against local Postgres (Lifecycle over the Pg adapters).
#[cfg(all(test, feature = "db-tests"))]
mod contract_tests;

// Shared test seeding for the db-tests modules.
#[cfg(all(test, feature = "db-tests"))]
pub(crate) mod test_support;

// ---- Curated public surface ----

pub use error::{ErrorClass, LifecycleError, RuntimeError};
pub use outcome::{Diagnostics, LifecycleOutcome};
pub use runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
pub use types::{
    ArchiveRetention, CorrelationMarker, IntentEffect, ObservedStatus, OperationGeneration,
    OperationKind, RuntimeEffect,
};

pub use orchestrator::{
    AgentStopPort, IntentWrite, Lifecycle, LifecycleEventSink, LifecycleLock, LifecycleStore,
    LockGuard, StoreError, SystemWallClock, TaskIntent, WallClock,
};

// The concrete port adapters `AppState` assembles into a `Lifecycle`.
pub(crate) use agent_stop::GrpcAgentStop;
pub(crate) use dao::PgLifecycleStore;
pub(crate) use event_sink::OutboxEventSink;
pub(crate) use lock::PgLifecycleLock;
pub(crate) use runtime_bridge::{ProvisionDeps, RuntimeBridge};
