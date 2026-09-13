//! Docker sandbox provider (band 7).
//!
//! The Docker Engine runtime behind the `moatless-sandbox-control` seams:
//! manages a dedicated container per task on a shared Docker network, resolved
//! by container name as hostname for gRPC. Implements the control band's ports
//! (`SandboxRuntime`, `FleetObserver`, status reads) and names `bollard` freely
//! — legal here, invisible to band 6 above. The backend composition root wires
//! this into `AppState`, never named here; sibling `moatless-sandbox-k8s` (same
//! band) is never named from here.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod client;
mod container_builder;
pub mod container_watcher;
pub mod fleet_observer;
mod naming;
pub mod provider;
mod status_resolver;
