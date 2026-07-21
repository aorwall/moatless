//! Durable lifecycle intent and orchestration for a Task's Sandbox.
//!
//! A deep module: callers identify a Task and request a lifecycle transition;
//! the module resolves intent, persists it before any runtime effect,
//! serializes per Task, invokes the runtime adapter behind the
//! [`runtime::SandboxRuntime`] seam, records bookkeeping, publishes
//! operation-level events, and returns a typed [`outcome::LifecycleOutcome`].
//! Callers never supply a `ProvisionRequest`, repositories, event publishers,
//! or the runtime adapter.
//!
//! This module realizes migration step 1 of
//! `specs/sandbox/04-lifecycle-architecture.md` ("Introduce lifecycle … add
//! recording adapter tests"). It contains the full typed surface, the runtime
//! seam, and the load-bearing decision logic as pure, exhaustively-tested
//! functions:
//!
//! - [`convergence`] — the reconciler's one normalized desired/observed map.
//! - [`classify`] — the watcher's four-case observation classification.
//! - [`marker`] — per-class correlation-marker acknowledgement.
//! - [`wake`] — message-wake causal precedence.
//! - [`events`] — deterministic operation-generation event ids + acceptance /
//!   completion / infrastructure derivation.
//! - [`attempt`] — the leader-local attempt cap and backoff.
//! - [`orchestrator`] — the [`Lifecycle`] type wiring the above over ports.
//!
//! The durability (marker acknowledgement, completion-fact persistence),
//! event-deduplication (deterministic ids over the transactional outbox), and
//! strict-removal (observed absence + archive obligation) rules are treated as
//! load-bearing and are the subject of the tests in each submodule.
//!
//! Steps 2–4 (migrating the caller sites, adding the persisted marker/retention
//! columns, threading `RuntimeOperationContext` through the K8s/Docker
//! adapters, standing up the desired-state reconciler, renaming
//! `SandboxProvider` → `SandboxRuntime`, and deleting the shallow paths) plug
//! into the ports this module defines.

pub mod attempt;
pub mod classify;
pub mod convergence;
pub mod error;
pub mod events;
pub mod marker;
pub mod orchestrator;
pub mod outcome;
pub mod runtime;
pub mod types;
pub mod wake;

// ---- Curated public surface ----

pub use error::{ErrorClass, LifecycleError, RuntimeError};
pub use outcome::{Diagnostics, LifecycleOutcome};
pub use runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
pub use types::{
    ArchiveRetention, CorrelationMarker, IntentEffect, ObservedStatus, OperationGeneration,
    OperationKind, RuntimeEffect,
};

pub use orchestrator::{
    AgentStopPort, Clock, Lifecycle, LifecycleEventSink, LifecycleLock, LifecycleStore, LockGuard,
    StoreError, SystemClock, TaskIntent,
};
