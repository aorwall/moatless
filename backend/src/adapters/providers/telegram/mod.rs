//! Telegram Bot API adapter.

mod access;
pub(crate) mod adapter;
mod api;
mod auth;
mod config;
pub(crate) mod delivery;
mod external_id;
pub(crate) mod handlers;
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
