//! Slack Adapter: Slack webhook handling, Web API, and outbound delivery.
//!
//! The backend remains the composition root and reaches this crate through a
//! permanent facade module (`crate::adapters::providers::slack`), so the server
//! band keeps naming its existing paths unchanged.

// Allow unwrap/expect in test code. Production code still enforces the deny.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod access;
pub mod adapter;
mod api;
mod auth;
mod config;
mod context;
pub mod delivery;
mod external_id;
mod format;
pub mod handlers;
mod inbound_files;
mod ingress;
mod outbound;
mod presentation;
mod reactions;
mod signature;
mod subject;
mod text;
mod webhook;
