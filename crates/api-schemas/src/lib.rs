//! Shared API request/response types for backend and CLI.
//!
//! This crate provides the canonical set of API DTOs (Data Transfer Objects)
//! used across both the Rust backend (with `utoipa` OpenAPI support) and the
//! CLI client (serde-only).
//!
//! # Feature flags
//!
//! - `openapi` — enables `utoipa::ToSchema` / `utoipa::IntoParams` derives

// reason: test code may unwrap/expect freely; production code keeps the workspace `deny`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod adapters;
pub mod agent;
pub mod auth;
pub mod bindings;
pub mod codex;
pub mod enums;
pub mod error;
pub mod feedback;
pub mod file_storage;
pub mod github;
pub mod linear;
pub mod loops;
pub mod message;
pub mod plugin;
pub mod repository;
pub mod sandbox;
pub mod setup;
pub mod shared;
pub mod tag;
pub mod task;
pub mod task_connections;
pub mod team;
pub mod user;
pub mod workspace;
