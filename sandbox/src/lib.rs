// Sandbox uses unsafe { std::env::set_var() } during single-threaded init phase.
#![allow(unsafe_code)]

//! Moatless Sandbox — library target.
//!
//! Re-exports all modules so that both the binary (`main.rs`) and
//! integration tests (`tests/`) can access them.

pub mod agent;
pub mod backend_client;
pub mod config;
pub mod error;
pub mod events;
pub mod files;
pub mod grpc_server;
pub mod init;
pub mod state;
