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

pub mod auth;
pub mod channel;
pub mod command;
pub mod config;
pub mod db;
pub mod error;
pub mod event_handler;
pub mod events;
pub mod extractors;
pub mod feature_flags;
pub mod figma;
pub mod leader;
pub mod file_storage;
pub mod message;
pub mod metrics;
pub mod plugin_repo;
pub mod provider_settings;
pub mod repository;
pub mod sandbox;
pub mod scope;
pub mod shared_models;
pub mod sse;
pub mod startup;
pub mod state;
pub mod tag;
pub mod task;
pub mod task_event;
pub mod team;
