//! Kubernetes-specific sandbox implementation.
//!
//! Contains all K8s-specific code: client, pod building, PVC,
//! service, pod reading, and status resolution.

pub mod auto_provision;
pub mod client;
pub mod deployment;
pub mod ingress;
pub mod leader_election;
pub mod naming;
pub mod pod_builder;
pub mod pod_reader;
pub mod pod_status;
pub mod pod_watcher;
pub mod provider;
pub mod pvc;
pub mod service;
pub mod status_resolver;
pub mod storage;
