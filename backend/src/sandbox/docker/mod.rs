//! Docker-based sandbox implementation.
//!
//! Manages sandbox containers via the Docker Engine API (bollard crate).
//! Each task gets a dedicated Docker container on a shared Docker network.
//! Communication between backend and sandbox uses gRPC, resolved by
//! container name as hostname on the Docker network.

pub mod client;
pub mod container_builder;
pub mod container_watcher;
pub mod fleet_observer;
pub mod naming;
pub mod provider;
pub mod status_resolver;
