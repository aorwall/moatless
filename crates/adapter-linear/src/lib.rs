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

/// The name Linear knows an actor by, for the messages they write.
///
/// Linear's webhook payloads default every actor field, so an event that names
/// nobody arrives with an empty name rather than no actor at all.
fn actor_handle(actor: Option<&webhook_payloads::LinearActor>) -> Option<String> {
    actor.and_then(|actor| moatless_loops::adapters::models::author_handle(&actor.name))
}

#[cfg(test)]
mod tests {
    use super::{actor_handle, webhook_payloads::LinearActor};

    fn actor(name: &str) -> LinearActor {
        LinearActor {
            id: "user-uuid".to_string(),
            name: name.to_string(),
            email: None,
        }
    }

    /// An event with no actor at all, and one whose actor was defaulted into
    /// existence by an empty payload, both name nobody.
    #[test]
    fn an_event_that_names_nobody_yields_no_handle() {
        assert_eq!(actor_handle(Some(&actor("Ada"))).as_deref(), Some("Ada"));
        assert_eq!(actor_handle(Some(&actor(""))), None);
        assert_eq!(actor_handle(None), None);
    }
}
