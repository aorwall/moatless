//! Permanent facade over the sandbox crates.
//!
//! `grpc_server` is the exception: it is a module of this crate, not a
//! re-export. It is the sandbox's inbound RPC surface — the same task, message,
//! event and file-storage work the REST routes do, over a different transport —
//! so it belongs at the edge with the other transports rather than inside the
//! lifecycle crate. It reaches back down into `event_dedup`, `jwt`, `metrics`,
//! `secrets` and `storage` through this facade.
//!
//! See `AGENTS.md` for what each crate owns, the band split, and the
//! composition root.

pub use moatless_sandbox_control::{
    auto_provision, event_dedup, file_handlers, handlers, jwt, lifecycle, notify, preflight, routes,
    secrets, workspace_snapshot,
};
pub use moatless_sandbox_runtime::{
    access, client, error, live_status, metrics, models, schemas, server_runner, status_reader,
    storage,
};

pub mod grpc_server;

pub use moatless_sandbox_docker as docker;
pub use moatless_sandbox_k8s as k8s;
