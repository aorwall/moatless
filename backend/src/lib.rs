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
pub mod adapters;
pub mod agent;
pub mod agent_event;
pub mod auth;
pub mod bindings;
pub mod compose;
pub mod config;
pub mod credentials;
pub mod db;
pub mod error;
pub mod event_handler;
pub mod event_ingress;
pub mod events;
pub mod extractors;
pub mod feature_flags;
pub mod feedback;
pub mod file_storage;
pub mod git_provider;
pub mod github_client;
pub mod leader;
pub mod loops;
pub mod message;
pub mod metrics;
pub mod plugin;
pub mod provider_settings;
pub mod repository;
pub mod sandbox;
pub mod scope;
pub mod secret;
pub mod setup;
pub mod shared_models;
pub mod skill;
pub mod sse;
pub mod startup;
pub mod state;
pub mod supervisor;
pub mod t3code;
pub mod tag;
pub mod task;
pub mod task_activities;
pub mod task_connections;
pub mod task_event;
pub mod task_listing;
pub mod task_turn;
pub mod team;
pub mod workspace;

#[cfg(test)]
pub(crate) mod test_support;
