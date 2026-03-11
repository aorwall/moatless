#![allow(clippy::print_stdout, clippy::print_stderr)]
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
