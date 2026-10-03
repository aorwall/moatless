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

//! Lowest-band backend crate: configuration, the database pool and migrations,
//! the error and event vocabularies, and the core-clean scope and shared-model
//! vocabularies every higher band depends on.

pub mod agent_catalog;
pub mod config;
pub mod db;
mod decision_config;
pub mod error;
pub mod events;
pub mod feature_flags;
pub mod file_storage;
pub mod forgejo_client;
pub mod git_provider;
pub mod github_client;
pub mod http_auth;
pub mod http_config;
pub mod leader;
pub mod metrics;
pub mod secret_keys;

pub mod change_feed;
pub mod db_enum;

pub mod sandbox_jwt;
mod user_agent;
pub mod workspace_repository;

pub use user_agent::USER_AGENT;
