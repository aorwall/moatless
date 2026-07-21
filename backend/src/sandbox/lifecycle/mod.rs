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
//! acknowledgement), [`wake`] (message-wake causal precedence), [`events`]
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
pub(crate) mod wake;

pub(crate) mod error;
pub(crate) mod events;
pub(crate) mod orchestrator;
pub(crate) mod outcome;
pub(crate) mod runtime;
pub(crate) mod types;

// Infrastructure adapters implementing the ports. Public so `AppState`/`main`
// construct them and integration tests exercise them against a live Postgres.
pub mod dao;

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

pub use dao::PgLifecycleStore;
