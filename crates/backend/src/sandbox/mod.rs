//! Permanent facade over the three sandbox crates.
//!
//! The whole sandbox subsystem now lives in banded crates; the backend
//! re-exports them here so higher bands still in the backend keep naming
//! `crate::sandbox::…`. Explicit named re-exports (never a glob) so the crate
//! census can resolve edges through the facade. The backend stays the
//! composition root that wires these ports into `AppState`.
//!
//! - `moatless-sandbox-runtime` (band 5) — what a runtime implements and the
//!   machinery it reuses, re-exported by name below.
//! - `moatless-sandbox-control` (band 6) — the orchestrator that drives a
//!   runtime, re-exported by name below.
//! - `moatless-sandbox-k8s` / `moatless-sandbox-docker` (band 7) — the two
//!   container runtimes, aliased below as `k8s` / `docker`. They depend downward
//!   on the control crate and implement its ports.
//!
//! `grpc_server` is the exception: it is a module of this crate, not a
//! re-export. It is the sandbox's inbound RPC surface — the same task, message,
//! event and file-storage work the REST routes do, over a different transport —
//! so it belongs at the edge with the other transports rather than inside the
//! lifecycle crate. It reaches back down into `event_dedup`, `jwt`, `metrics`,
//! `secrets` and `storage` through this facade.
//!
//! See `AGENTS.md` for the guide split across the three crates.

pub use moatless_sandbox_control::{
    auto_provision, event_dedup, file_handlers, handlers, jwt, lifecycle, notify, preflight, routes,
    secrets,
};
pub use moatless_sandbox_runtime::{
    access, client, error, live_status, metrics, models, schemas, server_runner, status_reader,
    storage,
};

pub mod grpc_server;

pub use moatless_sandbox_docker as docker;
pub use moatless_sandbox_k8s as k8s;
