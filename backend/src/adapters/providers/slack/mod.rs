//! Slack adapter — webhook handling, Web API, and signature verification.

mod access;
pub(crate) mod adapter;
mod api;
mod auth;
mod config;
mod context;
/// `pub`, unlike `github_pr`'s: `backend/tests/adapter_inbound_delivery_test.rs`
/// registers this handler by hand, and an integration test is a separate crate
/// that `pub(crate)` puts out of reach. It narrows the day that test stops
/// naming it.
pub mod delivery;
mod external_id;
mod format;
pub(crate) mod handlers;
mod ingress;
mod outbound;
mod reactions;
mod signature;
mod subject;
mod text;
mod webhook;

pub(crate) use auth::resolve_bot_user_id;
