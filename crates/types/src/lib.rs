//! Shared type definitions for Moatless.
//!
//! Every type here is pure data: no I/O, no behaviour beyond what the data
//! itself can answer.
//!
//! Admission and dependency constraints are enforced by `tooling/arch-rules`.
//!
//! No type here carries `#[schema(as = …)]`, so its OpenAPI schema name
//! derives from the type name, mirrored in TypeScript under
//! `packages/api-client/src/sandbox/model/`. A rename or move must leave
//! `openapi-specs.json` byte-identical; `#[serde(rename)]` pins travel with it.

// Test code asserts a parse succeeded; production code keeps the workspace deny.
#![cfg_attr(
    test,
    expect(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "tests fail by panicking"
    )
)]

pub mod access_control;
pub mod adapter;
pub mod agent;
pub mod attribution;
pub mod auth;
pub mod credential_key;
pub mod event;
pub mod feedback;
pub mod git_host;
pub mod http;
pub mod issue_tracker;
pub mod loops;
pub mod memory;
pub mod message;
pub mod model;
pub mod pull_request_conversation;
pub mod pull_request_key;
pub mod sandbox;
pub mod secret;
pub mod store;
mod string_enum;
pub mod task;
pub mod user;

pub mod activity;
pub mod codex;
pub mod running;
pub mod ui_rpc;
