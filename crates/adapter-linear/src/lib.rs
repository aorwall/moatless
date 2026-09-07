//! Linear adapter — Agents-first integration (mention / assignment driven
//! `AgentSessionEvent` webhooks). Maps Linear workspaces to integration
//! connections and sessions to task bindings.
//!
//! The backend remains the composition root and reaches this crate through a
//! permanent facade module (`crate::adapters::providers::linear`), so the server
//! band keeps naming its existing paths unchanged.

// Allow unwrap/expect in test code. Production code still enforces the deny.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//!
//! Four modules face outward, and each has a caller that names it:
//!
//! - [`adapter`] — the type the registry holds.
//! - [`delivery`] — the five kinds and their registration, named by the
//!   delivery worker and by `backend/tests/`.
//! - [`handlers`] — the webhook route.
//! - [`activity`] — the only module the event pipeline imports (Decision 6).
//!
//! [`ingress`] is the fifth, and it is exported for one reason: `receive` is
//! the seam this Adapter's ingress tests drive, and `backend/tests/` is a
//! separate crate — the only tier that can build an `AppState`.
//!
//! Everything else is an internal of this crate. [`active_turns`] and
//! [`api_routes`] are public because `state.rs` and `adapters/routes.rs` name
//! them through the backend facade; the rest is reachable only from inside
//! Linear.

pub mod active_turns;
pub mod activity;
pub mod adapter;
mod agent_api;
pub mod api_routes;
mod auth;
mod close;
mod comments;
mod config;
pub mod delivery;
pub mod event_subscriber;
mod external_id;
pub mod handlers;
pub mod ingress;
pub mod installation;
mod issues;
mod notifications;
mod sessions;
mod signature;
mod status;
mod subject;
mod text;
mod webhook;
mod webhook_payloads;

pub use subject::SUBJECT_KIND_ISSUE;

mod subscription_rules;
mod task_links;

mod issue_commands;
mod reference_resolution;
