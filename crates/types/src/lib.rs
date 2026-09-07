//! Shared type definitions for Moatless.
//!
//! Every type here is pure data: no I/O, no behaviour beyond what the data
//! itself can answer.
//!
//! Admission is enforced by `tooling/arch-rules`; see `crates/types/AGENTS.md`.

// Test code asserts a parse succeeded; production code keeps the workspace deny.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod string_enum;
pub(crate) use string_enum::string_enum;
pub mod access_control;
pub mod adapter;
pub mod agent;
pub mod auth;
pub mod event;
pub mod feedback;
pub mod loops;
pub mod message;
pub mod repository;
pub mod sandbox;
pub mod secret;
pub mod store;
pub mod task;
pub mod user;
pub mod workspace;

pub mod activity;
pub mod codex;
pub mod plugin;
pub mod running;
pub mod ui_rpc;
