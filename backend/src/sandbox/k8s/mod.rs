//! Kubernetes-specific sandbox implementation.
//!
//! Contains all K8s-specific code: client, pod building, PVC, secret,
//! service, health checks, and status resolution.

pub mod auto_provision;
pub mod client;
pub mod deployment;
pub mod health;
pub mod ingress;
pub mod leader_election;
pub mod naming;
pub mod pod_builder;
pub mod pod_status;
pub mod pod_watcher;
pub mod provider;
pub mod pvc;
pub mod secret;
pub mod service;
pub mod status_resolver;
