//! Linear adapter — Agents-first integration (mention / assignment driven
//! `AgentSessionEvent` webhooks). Maps Linear workspaces to integration
//! connections and sessions to task bindings.
//!
//! The backend remains the composition root and reaches this crate through a
//! permanent facade module (`crate::adapters::providers::linear`), so the
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
//!
//! Five modules face outward, each with a caller that names it: [`adapter`]
//! (the registry), [`delivery`] (the worker and `backend/tests/`), [`handlers`]
//! (the webhook route), [`activity`] (the event pipeline, per Decision 6) and
//! [`issue_tracker`] (T3's Issues page). [`ingress`] is exported only because
//! `receive` is the seam `backend/tests/` drives to build an `AppState`.
//!
//! Everything else is internal; [`active_turns`] and [`api_routes`] are public
//! only because `state.rs` and `adapters/routes.rs` name them through the
//! backend facade.

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
pub mod issue_tracker;
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

mod description_readback;
mod issue_commands;
mod login_annotation;
mod reference_resolution;
