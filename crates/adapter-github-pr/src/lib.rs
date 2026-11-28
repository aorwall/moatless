//! GitHub PR Adapter: GitHub webhook handling, PR workflows, replies, and
//! reactions.
//!
//! The backend remains the composition root and reaches this crate through a
//! permanent facade module (`crate::adapters::providers::github_pr`), so the
//! server band keeps naming its existing paths unchanged.

// Allow unwrap/expect in test code. Production code still enforces the deny.
#![cfg_attr(
    test,
    expect(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )
)]

pub mod adapter;
mod api;
pub mod api_routes;
mod auth;
mod check_run;
pub mod check_run_trigger;
mod config;
pub mod delivery;
mod external_id;
pub mod handlers;
mod ingress;
mod merge_conflict;
mod merged;
mod outbound;
pub mod own_event;
mod post_merge;
mod reaction_api;
mod reactions;
mod read_guard;
mod rejection;
mod reply_keys;
pub mod router;
mod routing;
mod signals;
mod signature;
mod status;
mod subject;
#[cfg(test)]
mod test_support;
mod text;
mod webhook;

pub use auth::pull_request_credits;
pub use ingress::{DEFAULT_BRANCH_PUSH, REPOSITORY_CHANGE};
pub use text::{PullRequestCredits, credit_driver, credit_pull_request};

mod mutations;
pub mod pull_request_conversation;
pub mod pull_request_lookup;
pub mod pull_request_search;
pub mod pull_request_writes;
