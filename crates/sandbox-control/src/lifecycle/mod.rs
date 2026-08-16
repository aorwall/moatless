//! Durable lifecycle intent and orchestration for a Task's Sandbox.
//!
//! A deep module: callers identify a Task and request a lifecycle transition;
//! the module resolves intent, persists it before any runtime effect,
//! serializes per Task, invokes the runtime adapter behind the
//! [`SandboxRuntime`] seam, records bookkeeping, publishes operation-level
//! events, and returns a typed [`LifecycleOutcome`]. Callers never supply a
//! provision request, DAOs, an event publisher, or the runtime adapter
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
//!   `sandbox:{task}:{operation_generation}:{fact}` over idempotent persistence,
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

// Decision modules driven by the desired-state reconciler (wired in `main`).
// `convergence`, `reconciler`, `classify`, `watcher`, `observe`, `orchestrator`,
// `outcome`, `runtime`, and `types` are `pub` (not `pub(crate)`) because the
// band-7 provider crates (`sandbox-k8s`, `sandbox-docker`, still in the backend)
// reach specific items in them across the facade — the pub-widening the carve
// pays. Widening the module only exposes its `pub` items; the rest stay
// `pub(crate)`, so a lifecycle mutation still cannot bypass `Lifecycle`.
pub(crate) mod attempt;
pub mod convergence;
pub(crate) mod reconcile;
pub mod reconciler;
pub(crate) mod wake;

// Watcher four-case classification and reconciler-owned marker acknowledgement.
pub mod classify;
pub(crate) mod marker;
pub mod watcher;

pub(crate) mod error;
pub(crate) mod events;
pub mod orchestrator;
pub mod outcome;
pub(crate) mod plan;
pub mod runtime;
pub mod types;

// Infrastructure adapters implementing the ports, `pub(crate)` and constructed
// by `AppState` when it builds the `Lifecycle` handle.
pub(crate) mod agent_stop;
pub(crate) mod dao;
pub(crate) mod event_sink;
pub(crate) mod lock;
pub mod observe;
pub(crate) mod workspace_archive;
pub(crate) mod workspace_sync;

// The lifecycle contract against local Postgres (Lifecycle over the Pg adapters).
#[cfg(all(test, feature = "db-tests"))]
mod contract_tests;

// Shared test seeding for the db-tests modules.
#[cfg(all(test, feature = "db-tests"))]
pub(crate) mod test_support;

// ---- Curated surface ----
//
// `pub` is the deep module's cross-crate contract: the `Lifecycle` handle, its
// typed outcomes/errors, and the reconciler surface `main` assembles. Everything
// the module is merely built *over* — the ports, the runtime seam, and the
// correlation internals — is `pub(crate)`; no caller outside this crate names it.

pub use error::{ErrorClass, LifecycleError, RuntimeError};
pub use orchestrator::Lifecycle;
pub use outcome::{Diagnostics, LifecycleOutcome};
pub use runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
pub use types::{
    ArchiveRetention, IntentEffect, ObservedStatus, OperationGeneration, OperationKind,
    RuntimeEffect,
};

// The desired-state reconciler and its gRPC notifier, assembled by
// `compose::Ports::build_reconciler` and spawned by `main`. The provider
// fleet-observer adapters live in `sandbox::k8s` and `sandbox::docker`.
pub use observe::{GrpcMemberRegistry, GrpcNotify};
pub use reconciler::{
    FleetObserver, MemberRegistryPort, NotifyPort, Reconciler, ReconcilerConfig, ReconcilerDeps,
    RuntimeObservation,
};

// The system clock and concrete port adapters the composition root assembles
// into a `Lifecycle`/`Reconciler`. These are `pub` because the composition root
// (`backend/src/compose.rs`, `state.rs`) now names them across the crate
// boundary — the pub-widening the carve pays for: `pub(crate)` cannot span two
// crates, so an item the backend builds must be `pub`. They remain excluded from
// this module's *curated* contract — a caller other than the composition root
// has no business naming a raw port adapter — but visibility follows real
// consumers, and the real consumer is now in another crate.
pub use agent_stop::GrpcAgentStop;
pub use dao::PgLifecycleDao;
pub use event_sink::OutboxEventSink;
pub use lock::PgLifecycleLock;
pub use orchestrator::SystemWallClock;
pub use plan::ProvisionDeps;
pub use workspace_archive::S3WorkspaceArchive;
pub use workspace_sync::GrpcWorkspaceSync;
