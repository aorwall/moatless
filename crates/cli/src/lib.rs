#![allow(clippy::print_stdout, clippy::print_stderr)]
// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! `moatless-cli` — library for the Moatless backend API client.
//!
//! Exposes [`MoatClient`], [`Config`], and typed command modules so that
//! integration tests (and other crates) can drive the API without duplicating
//! client construction logic.

pub mod auth;
pub mod client;
pub mod commands;
pub mod config;
pub mod credentials;
pub mod events;
pub mod output;
