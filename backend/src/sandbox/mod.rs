//! Sandbox operations module.
//!
//! Provides internal API for managing sandbox pods, PVCs, secrets,
//! and services. The `k8s` submodule contains Kubernetes-specific
//! implementation details.

pub mod access;
pub mod auto_provision;
pub mod client;
pub mod docker;
pub mod error;
pub mod event_dedup;
pub mod file_handlers;
pub mod grpc_server;
pub mod handlers;
pub mod jwt;
pub mod k8s;
pub mod lifecycle;
pub mod live_status;
pub mod metrics;
pub mod models;
pub mod notify;
pub mod preflight;
pub mod routes;
pub mod schemas;
pub mod status_reader;
pub mod secrets;
pub mod server_runner;
