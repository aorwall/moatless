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

pub mod adapter_apps;
pub mod adapter_installations;
pub mod adapters;
pub mod agent;
pub mod agent_event;
pub mod auth;
pub mod bindings;
pub mod compose;
pub mod credential_sync;
pub mod credentials;
pub mod event_handler;
pub mod event_ingress;
pub mod extractors;
pub mod feedback;
pub mod feature_flags;
pub mod file_storage;
pub mod loops;
pub mod message;
pub mod plugin;
pub mod provider_settings;
pub mod repository;
pub mod repository_sync;
pub mod sandbox;
pub mod secret;
pub mod skill;
pub mod sse;
pub mod startup;
pub mod state;
pub mod supervisor;
pub mod tag;
pub mod t3code;
pub mod task;
pub mod task_activities;
pub mod task_connections;
pub mod task_event;
pub mod task_listing;
pub mod task_turn;
pub mod team;
pub mod workspace;
