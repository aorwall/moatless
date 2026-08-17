//! GitHub PR Adapter: GitHub webhook handling, PR workflows, replies, and reactions.
//!
//! The backend remains the composition root and reaches this crate through a
//! permanent facade module
//! (`crate::adapters::providers::github_pr`), so the server band keeps naming
//! its existing paths unchanged.

// Allow unwrap/expect in test code. Production code still enforces the deny.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod adapter;
mod api;
pub mod api_routes;
mod auth;
mod config;
pub mod delivery;
mod external_id;
pub mod handlers;
mod ingress;
mod merge_conflict;
mod outbound;
mod post_merge;
mod reactions;
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
