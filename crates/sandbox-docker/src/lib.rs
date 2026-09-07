//! Docker sandbox provider (band 7).
//!
//! The Docker Engine runtime behind the `moatless-sandbox-control` seams:
//! manages a dedicated container per task on a shared Docker network, resolved
//! by container name as hostname for gRPC. It implements the control band's
//! ports (`SandboxRuntime`, `FleetObserver`, status reads) and names `bollard`
//! freely — that SDK is legal in a provider band and invisible to the control
//! band (6) above, which depends only on the ports. The backend composition
//! root wires this provider into `AppState`; the crate names no concrete
//! `AppState`. The sibling `moatless-sandbox-k8s` crate is the same band and is
//! never named from here.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod client;
mod container_builder;
pub mod container_watcher;
pub mod fleet_observer;
mod naming;
pub mod provider;
mod status_resolver;
