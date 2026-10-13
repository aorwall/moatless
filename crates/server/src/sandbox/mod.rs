//! Permanent facade over the sandbox crates: add no sandbox logic here.
//! Start in `moatless-sandbox-control` for intent, events, or the
//! reconciler; each crate's own module doc holds its rules.
//!
//! Keep every re-export explicit and named, never a glob — the crate census
//! resolves an edge through this facade only when the name is written out.
//!
//! `grpc_server` is the exception: a module of this crate, not a re-export,
//! since it's the sandbox's inbound RPC surface and reaches back into
//! `event_dedup`, `metrics`, `secrets` and `storage` directly.

pub use moatless_sandbox_control::{
    auto_provision, event_dedup, file_handlers, handlers, lifecycle, notify, preflight, routes,
    secrets, workspace_repository, workspace_snapshot,
};
pub use moatless_sandbox_runtime::{
    access, client, container_status, error, live_status, metrics, provisioning, schemas,
    status_reader, storage,
};

pub mod grpc_server;

pub use moatless_sandbox_docker as docker;
pub use moatless_sandbox_k8s as k8s;
pub use moatless_sandbox_local as local;

pub(crate) mod join;
