//! Shared wire contracts for domains outside the Workspace types crate.

#![cfg_attr(
    test,
    expect(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )
)]

pub mod adapters;
pub mod agent;
pub mod artifact;
pub mod auth;
pub mod bindings;
pub mod codex;
pub mod collection;
pub mod decisions;
pub mod enums;
pub mod error;
pub mod feedback;
pub mod file_storage;
pub mod github;
pub mod linear;
pub mod loops;
pub mod memory;
pub mod message;
pub mod patch;
pub mod provider_apps;
pub mod sandbox;
pub mod setup;
pub mod tag;
pub mod task;
pub mod user;
