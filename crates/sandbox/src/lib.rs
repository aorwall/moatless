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
//! Re-exports all modules so that both the binary (`main.rs`) and integration
//! tests (`tests/`) can access them.

pub mod agent;
pub mod backend_client;
pub mod commands;
pub mod config;
pub mod credentials;
pub mod error;
pub mod events;
pub mod files;
mod git_push_watcher;
pub mod grpc_server;
pub mod init;
pub mod outbox;
pub mod pod_processes;
pub mod preview_proxy;
pub mod server_control;
pub mod shutdown;
pub mod state;
pub mod supervisor;
pub mod terminal;

pub(crate) mod transcript_sync;
pub mod workspace_sync;
pub mod writer_admission;

pub mod app;
mod compose;
mod health;
mod task_context;

mod terminal_grpc;
