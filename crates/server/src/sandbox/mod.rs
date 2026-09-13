//! Permanent facade over the sandbox crates. Add no sandbox logic here; put it
//! in one of the re-exported crates, behind that crate's band. Start in
//! `moatless-sandbox-control` for a change to sandbox intent, events, or the
//! reconciler; each crate's own module docs hold its rules.
//!
//! Keep every re-export explicit and named. Never write a glob: the crate
//! census resolves an edge through this facade to its definition site only
//! when the name is written out.
//!
//! `grpc_server` is the exception: it is a module of this crate, not a
//! re-export. It is the sandbox's inbound RPC surface — the same task, message,
//! event and file-storage work the REST routes do, over a different transport —
//! so it belongs at the edge with the other transports rather than inside the
//! lifecycle crate. It reaches back down into `event_dedup`, `metrics`,
//! `secrets` and `storage` through this facade, and names
//! `moatless_core::sandbox_jwt` directly.
//!
//! ## What the backend still owns
//!
//! Named modules under `crate::compose` build the concrete provider adapters
//! (`K8sSandboxAdapter`, `DockerSandboxAdapter`) and the `Lifecycle` handle;
//! `app.rs` builds the `Reconciler` directly from
//! `moatless_sandbox_control::compose::ReconcilerInputs`. The four re-exported
//! crates are generic over the ports — only the backend names the concrete
//! wiring.
//!
//! The unified task reaper, in `crate::reaper`, is the other thing the backend
//! owns: task-scoped stop and remove still route through `Lifecycle` in
//! `moatless-sandbox-control`.

pub use moatless_sandbox_control::{
    auto_provision, event_dedup, file_handlers, handlers, lifecycle, notify, preflight, routes,
    secrets, workspace_repository, workspace_snapshot,
};
pub use moatless_sandbox_runtime::{
    access, client, container_status, error, live_status, metrics, provisioning, schemas,
    server_runner, status_reader, storage,
};

pub mod grpc_server;

pub use moatless_sandbox_docker as docker;
pub use moatless_sandbox_k8s as k8s;

pub(crate) mod join;
