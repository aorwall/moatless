//! Permanent facade over the sandbox-control band (`moatless-sandbox-control`).
//!
//! The provider-agnostic half of the sandbox module now lives in its own crate;
//! the backend re-exports those modules here so higher bands still in the
//! backend keep naming `crate::sandbox::…`. Explicit named re-exports (never a
//! glob) so the crate census can resolve edges through the facade. The backend
//! stays the composition root that wires these ports into `AppState`.
//!
//! `k8s` and `docker` (band 7, the two container runtimes) stay backend-local
//! and reach the control band through this facade — they depend downward on the
//! crate, so they must not move into it.

pub use moatless_sandbox_control::{
    access, auto_provision, client, error, event_dedup, file_handlers, grpc_server, handlers, jwt,
    lifecycle, live_status, metrics, models, notify, preflight, resource_parse, routes, schemas,
    secrets, server_runner, status_reader, storage,
};

pub mod docker;
pub mod k8s;
