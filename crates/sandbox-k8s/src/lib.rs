//! Kubernetes sandbox provider (band 7): the runtime behind
//! `moatless-sandbox-control`'s ports (`SandboxRuntime`, `FleetObserver`,
//! status reads) — client, pod/PVC/service/network construction, pod reading
//! and watching, node archival, leader election, and status resolution.
//! `kube`/`k8s-openapi` are legal in this band and invisible above it.
//!
//! The `moatless.io/task-id` label is how almost every read finds its
//! resource; a resource created without it is invisible to cleanup and status.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod client;
mod deployment;
mod edge;
pub mod fleet_observer;
mod ingress;
pub mod leader_election;
mod naming;
mod networkpolicy;
pub mod node_archive;
mod owner_ref;
mod pod_builder;
mod pod_observation_policy;
mod pod_reader;
mod pod_status;
pub mod pod_watcher;
pub mod provider;
mod pvc;
mod rescue_pod;
pub mod sandbox_resources;
mod service;
pub mod status_resolver;
mod wait;

pub use ingress::warn_if_previews_unroutable;
