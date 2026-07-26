// Allow unwrap/expect/print in test code — production code still enforces the deny.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::print_stdout,
        clippy::print_stderr
    )
)]

//! Moatless Sandbox — library target.
//!
//! Re-exports all modules so that both the binary (`main.rs`) and
//! integration tests (`tests/`) can access them.

pub mod agent;
pub mod backend_client;
pub mod config;
pub mod credentials;
pub mod error;
pub mod events;
pub mod files;
pub mod grpc_server;
pub mod init;
pub mod outbox;
pub mod process_tree;
pub mod server_control;
pub mod shutdown;
pub mod state;
pub mod supervisor;
