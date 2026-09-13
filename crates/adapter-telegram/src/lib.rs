//! Telegram Adapter: Telegram Bot API inbound, polling, replies, and reactions.
//!
//! The backend remains the composition root and reaches this crate through a
//! permanent facade module (`crate::adapters::providers::telegram`), so the
//! server band keeps naming its existing paths unchanged.

// Allow unwrap/expect in test code. Production code still enforces the deny.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod access;
pub mod adapter;
mod api;
mod auth;
mod config;
pub mod delivery;
mod external_id;
pub mod handlers;
mod ingress;
mod outbound;
mod pairing;
mod poller;
mod reactions;
mod subject;
#[cfg(test)]
mod test_support;
mod webhook;
#[cfg(test)]
mod webhook_tests;

pub use poller::spawn_telegram_poller;
