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

pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod feature_flags;
pub mod git_provider;
pub mod github_client;
pub mod leader;
pub mod metrics;
pub mod scope;
pub mod secret_keys;
pub mod shared_models;
