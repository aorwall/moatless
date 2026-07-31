//! Linear adapter — Agents-first integration (mention / assignment driven
//! `AgentSessionEvent` webhooks). Maps Linear workspaces to integration
//! connections and sessions to task bindings.
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
//! Everything else is an internal of this module. [`active_turns`] and
//! [`api_routes`] are `pub(crate)` rather than private because `state.rs` and
//! `adapters/routes.rs` name them from elsewhere in the crate; the rest is
//! reachable only from inside `linear`.

pub(crate) mod active_turns;
pub(crate) mod activity;
pub mod adapter;
mod agent_api;
pub(crate) mod api_routes;
mod auth;
mod close;
mod comments;
mod config;
pub mod delivery;
mod external_id;
pub mod handlers;
pub mod ingress;
mod issues;
mod notifications;
mod sessions;
mod signature;
mod status;
mod subject;
mod text;
mod webhook;
mod webhook_payloads;

/// `task_bindings.subject_kind` for bindings that point to a Linear issue
/// (CLI-created or operator-initiated). Distinct from the existing
/// `linear_agent_session` kind, which routes replies through
/// `agentActivityCreate`; `linear_issue` routes through `commentCreate`.
pub const SUBJECT_KIND_ISSUE: &str = "linear_issue";
