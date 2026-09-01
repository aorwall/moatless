//! Durable lifecycle intent and orchestration for a Task's Sandbox.
//!
//! Callers identify a Task and request a transition; the module resolves intent,
//! persists it, serializes per Task, invokes the [`SandboxRuntime`] seam, and
//! returns a [`LifecycleOutcome`]. The curated re-exports below are the whole
//! surface — the decision and implementation submodules are `pub(crate)`, so a
//! lifecycle mutation cannot bypass [`Lifecycle`].
//!
//! Guarantees:
//! - **Intent before effect.** Desired state, stop reason, retention and the
//!   correlation marker are written atomically before the runtime effect, so a
//!   runtime failure leaves the requested target for recovery.
//! - **Deduplicated events.** Operation events use the id
//!   `sandbox:{task}:{operation_generation}:{fact}`, and a completion fact is
//!   persisted with the marker cleared in one transaction before publication.
//! - **Strict removal.** A remove reports success only on observed absence of
//!   every resource, PVC included; its marker is never age-cleared.
//! - **Correlation.** The operation generation rides every runtime mutation and
//!   watcher event, so superseded observations are told apart by identity rather
//!   than by the clock.
//!
//! Decision logic: [`convergence`], [`classify`], [`marker`], [`wake`],
//! [`reconcile`], [`events`], [`attempt`].

// Decision modules driven by the desired-state reconciler (wired in `main`).
// `reconciler`, `watcher`, `observe`, `orchestrator` and `error` are `pub` (not
// `pub(crate)`) because the band-7 provider crates (`sandbox-k8s`,
// `sandbox-docker`) reach specific items in them across the crate boundary.
// Widening the module only exposes its `pub` items; the rest stay
// `pub(crate)`, so a lifecycle mutation still cannot bypass `Lifecycle`.
//
// The four decision modules a runtime also reads — `convergence`, `classify`,
// `marker`, `types` — and the runtime seam itself live in
// `moatless-sandbox-runtime`.
pub(crate) mod attempt;
pub(crate) mod reconcile;
pub mod reconciler;
pub(crate) mod wake;

// Watcher four-case classification and reconciler-owned marker acknowledgement.
pub mod watcher;

pub mod error;
pub(crate) mod events;
pub mod orchestrator;
pub(crate) mod plan;

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
// typed error, and the reconciler surface `main` assembles. Everything the
// module is merely built *over* — the ports and the correlation internals — is
// `pub(crate)`; no caller outside this crate names it. The outcome a caller
// reads back is `moatless_sandbox_runtime::outcome::LifecycleOutcome`, named
// from that crate.

pub use error::LifecycleError;
pub use orchestrator::{Lifecycle, TaskIntent};

// The desired-state reconciler and its gRPC notifier, assembled by
// `compose::Ports::build_reconciler` and spawned by `main`. The provider
// fleet-observer adapters live in `sandbox::k8s` and `sandbox::docker`.
pub use observe::{GrpcMemberRegistry, GrpcNotify};
pub use reconciler::{
    MemberRegistryPort, NotifyPort, Reconciler, ReconcilerConfig, ReconcilerDeps,
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
