//! Durable lifecycle intent and orchestration for a Task's Sandbox.
//!
//! Callers identify a Task and request a transition; the module resolves
//! intent, persists it, serializes per Task, invokes the [`SandboxRuntime`]
//! seam, and returns a [`LifecycleOutcome`].
//! Guarantees: intent written before effect; events deduplicated by operation
//! generation; strict removal (absence of every resource, PVC included); and
//! correlation by generation rather than by the clock.

// `reconciler`, `watcher`, `observe` and `error` are `pub`, not
// `pub(crate)`: band-7 provider crates and the backend supervisor reach
// specific items in them across the crate boundary. Only their `pub`
// items widen; the rest stays `pub(crate)`, so a mutation still can't
// bypass `Lifecycle`.
pub(crate) mod attempt;
pub(crate) mod reconcile;
pub mod reconciler;
pub(crate) mod wake;

// Watcher four-case classification and reconciler-owned marker acknowledgement.
pub mod watcher;

pub mod error;
pub(crate) mod events;
pub mod ingress;
pub(crate) mod orchestrator;
pub(crate) mod plan;
pub mod token_generation;

// Infrastructure adapters implementing the ports, `pub(crate)` and constructed
// by `crate::compose` when it builds the `Lifecycle` handle.
pub(crate) mod agent_stop;
pub(crate) mod dao;
pub(crate) mod db;
pub(crate) mod event_sink;
pub mod observe;
pub(crate) mod workspace_archive;
pub(crate) mod workspace_sync;

#[cfg(all(test, feature = "db-tests"))]
mod contract_tests;

#[cfg(all(test, feature = "db-tests"))]
pub(crate) mod test_support;

// ---- Curated surface ----
//
// `pub` is the cross-crate contract: the `Lifecycle` handle, its typed
// error, and the reconciler surface `main` spawns. Everything merely built
// *over* — stores, ports, correlation internals — is `pub(crate)`;
// `crate::compose` is the only assembler a caller outside names.

pub use error::LifecycleError;
pub use ingress::{SandboxIdentity, SandboxIngress};
pub use orchestrator::{Lifecycle, OutstandingCommand, TaskIntent};
pub use token_generation::{DriverChange, TokenGenerationStore};

// The desired-state reconciler and its gRPC notifier. `crate::compose` builds
// the `Reconciler` and `main` spawns it; the notifier and member registry are
// built over the backend's connector. The provider fleet-observer adapters live
// in `sandbox::k8s` and `sandbox::docker`.
pub use observe::{GrpcMemberRegistry, GrpcNotify};
pub use reconciler::{MemberRegistryPort, NotifyPort, Reconciler, ReconcilerConfig};

// The provisioning knowledge the band-7 provider crates read, and the S3
// archive the backend builds over its own bucket configuration and hands to
// `crate::compose`.
pub use plan::ProvisionDeps;
pub use workspace_archive::S3WorkspaceArchive;
