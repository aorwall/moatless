//! Sandbox operations module.
//!
//! Provides internal API for managing sandbox pods, PVCs, secrets,
//! and services. The `k8s` submodule contains Kubernetes-specific
//! implementation details. Exposed on a separate port (8081) for
//! internal communication from the TypeScript backend.

pub mod error;
pub mod jwt;
pub mod k8s;
pub mod provider;
pub mod routes;
pub mod types;
