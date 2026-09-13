//! Kubernetes sandbox provider (band 7).
//!
//! The Kubernetes runtime behind the `moatless-sandbox-control` seams: client,
//! pod/PVC/service/network construction, pod reading and watching, node
//! archival, leader election, and status resolution. It implements the
//! control band's ports (`SandboxRuntime`, `FleetObserver`, status reads) and
//! names `kube`/`k8s-openapi` freely — those SDKs are legal in a provider band
//! and invisible to the control band above it, which depends only on the
//! ports. The backend composition root wires this provider into `AppState`;
//! the crate names no concrete `AppState`. The sibling `moatless-sandbox-docker`
//! crate is the same band and is never named from here.
//!
//! The `moatless.io/task-id` label is how almost every read finds a resource:
//! `pod_reader`, `pvc`, `ingress`, `service`, `edge` and `deployment` all
//! select on it. A resource created without it is invisible to cleanup and to
//! status.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod client;
mod deployment;
pub mod edge;
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
