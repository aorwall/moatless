//! Kubernetes sandbox provider (band 7).
//!
//! The Kubernetes runtime behind the `moatless-sandbox-control` seams: client,
//! pod/PVC/service/network construction, pod reading and watching, node
//! archival, leader election, and status resolution. It implements the
//! control band's ports (`SandboxRuntime`, `FleetObserver`, status reads) and
//! names `kube`/`k8s-openapi` freely — those SDKs are legal in a provider band
//! and invisible to the control band above it, which depends only on the
//! ports. The backend composition root wires this provider into `AppState`;
//! the crate names no concrete `AppState`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod client;
pub mod deployment;
pub mod edge;
pub mod fleet_observer;
pub mod ingress;
pub mod leader_election;
pub mod naming;
pub mod networkpolicy;
pub mod node_archive;
pub mod owner_ref;
pub mod pod_builder;
mod pod_observation_policy;
pub mod pod_reader;
pub mod pod_status;
pub mod pod_watcher;
pub mod provider;
pub mod pvc;
pub mod sandbox_resources;
pub mod service;
pub mod status_resolver;
pub mod wait;
