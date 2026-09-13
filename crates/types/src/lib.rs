//! Shared type definitions for Moatless.
//!
//! Every type here is pure data: no I/O, no behaviour beyond what the data
//! itself can answer.
//!
//! Admission and dependency constraints are enforced by `tooling/arch-rules`.
//!
//! No type here carries `#[schema(as = …)]`, so an OpenAPI schema name derives
//! from the type name alone, and `packages/api-client/src/sandbox/model/`
//! mirrors it in TypeScript. After a rename or a move, `task generate:spec`
//! must leave `openapi-specs.json` byte-identical. An explicit
//! `#[serde(rename)]` pin travels with its type.

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
pub mod git_host;
pub mod http;
pub mod loops;
pub mod message;
pub mod sandbox;
pub mod secret;
pub mod store;
pub mod task;
pub mod user;

pub mod activity;
pub mod codex;
pub mod running;
pub mod ui_rpc;
