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

//! Sandbox Controller library — re-exports all modules for integration tests.

pub mod access_settings;
pub mod adapter_installations;
pub mod adapters;
pub mod agent_harness_access;
pub mod app;
pub mod app_administration;
pub mod compose;
pub mod event_handler;
pub mod feature_flags;
pub mod file_storage;
pub mod git_proxy;
pub(crate) mod github_policy;
pub mod github_proxy;
mod http_trace;
pub mod model_gateway;
mod proxy;
pub mod reaper;
mod reaper_decisions;
pub mod repository_sync;
pub mod sandbox;
pub mod sandbox_driver;
pub mod secret_cli;
pub mod service_proxy;
pub mod sse;
pub mod startup;
pub mod state;
pub mod supervisor;
pub mod t3code;
