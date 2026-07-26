//! Durable admission for Linear agent-session provisioning.
//!
//! Provisioning the task behind a Linear `AgentSessionEvent.created` used to be
//! `tokio::spawn`ed: it selects a spawn rule (possibly fetching the issue),
//! creates a task, starts its sandbox, and writes the session binding — none of
//! which fits inside Linear's 10-second webhook window. The webhook returned
//! `200` as soon as the spawn was launched, so a deploy or a kill between the
//! acknowledgment and the task creation lost the session with no trace: Linear
//! considers a delivery acknowledged and does not send it again.
//!
//! It is now admitted into [`crate::adapters::work`] before the response is
//! returned. The admission contract lives in that module and is not restated
//! here; what this module owns is the Linear-specific half.
//!
//! # Idempotency key
//!
//! `event_id` is the *existing* [`dedup_key`](super::handlers) string —
//! `{event_type}:{action}:{resource_id}` with the documented fallback chain
//! ending at `webhook_id:webhook_timestamp`. It is deliberately not a new
//! scheme: Linear's `webhookId` is the stable webhook-*configuration* id,
//! identical on every delivery, so keying on it would collapse two distinct
//! sessions into one record; and deriving a second key beside the existing one
//! would let the same delivery dedup under one mechanism and not the other.
//!
//! # Relationship to the in-process dedup cache
//!
//! [`LinearAdapter`](super::adapter::LinearAdapter)'s 5-minute in-memory cache
//! stays, keyed on the same string. The two are authoritative for different
//! things:
//!
//! - The **in-memory cache** is authoritative for the whole dispatch, including
//!   the event types that are *not* admitted — `Issue`, `Comment`,
//!   `AppUserNotification`, and `AgentSessionEvent.prompted` all still run
//!   inline on the response path, and the cache is the only thing that collapses
//!   a Linear retry of those.
//! - The **durable unique index** is authoritative for the one admitted effect,
//!   session provisioning. It covers what a per-pod cache cannot: a retry
//!   landing on another replica, after a restart, or after the TTL.
//!
//! Because both use the same key, a delivery that slips past the cache still
//! dedups durably. Nothing the cache covered before is uncovered now.
//!
//! # Execution deadline
//!
//! Provisioning is the slow work this seam exists for, and
//! [`AdapterWorkPolicy::execution_timeout`](crate::adapters::work::AdapterWorkPolicy)
//! is 2 minutes. If it is exceeded the attempt is abandoned and retried, and
//! the retry converges on the binding guard in `provision_session_task` rather
//! than creating a second task. The uncovered window is a timeout landing
//! between task creation and that binding write; the spawned path lost the
//! session outright in the same window.

use moatless_backend_core::scope::ScopeContext;
use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::work::{
    AdapterWork, AdapterWorkFailure, AdapterWorkHandler, AdapterWorkHandlers, AdapterWorkRequest,
    AdmitError,
};
use crate::state::AppState;

use super::webhook_payloads::AgentSession;

/// Provider label on every record this module admits.
pub const PROVIDER: &str = "linear";

/// Provision the task behind a newly created Linear agent session.
pub const SESSION_PROVISION: &str = "linear.session_provision";

/// Register the Linear work handlers on the process-wide registry.
pub fn register(handlers: &mut AdapterWorkHandlers, state: &AppState) {
    handlers.register(
        SESSION_PROVISION,
        Arc::new(SessionProvisionHandler(state.clone())),
    );
}

/// Why a Linear webhook did not reach a normal outcome.
///
/// The variants get opposite transport treatment: [`Self::NotAdmitted`] means
/// nothing was written, so Linear must be told to send the delivery again,
/// while [`Self::Ingress`] keeps the long-standing behaviour of a logged,
/// metered `200`.
#[derive(Debug, thiserror::Error)]
pub enum LinearWebhookError {
    /// Durable admission did not commit.
    #[error("Linear webhook work was not admitted")]
    NotAdmitted(#[from] AdmitError),
    /// Everything the dispatch path itself rejected or failed on.
    #[error(transparent)]
    Ingress(#[from] AdapterError),
}

/// Everything the provisioning handler needs to redo the work from scratch.
///
/// The connection is referenced by id rather than snapshotted, so a webhook
/// secret never lands in a table retained for weeks and a record executed
/// minutes later sees the connection as it is. The session, by contrast, is
/// carried verbatim: it is the provider's own event payload and cannot be
/// re-fetched from anywhere once the request body is gone.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SessionProvisionWork {
    pub(super) connection_id: String,
    pub(super) session: AgentSession,
}

/// Failures this module raises on its own behalf.
#[derive(Debug, thiserror::Error)]
enum LinearWorkError {
    #[error("adapter connection {0} no longer exists")]
    ConnectionGone(String),
    #[error("adapter work payload does not match its kind")]
    Payload(#[source] serde_json::Error),
}

/// Build the admission request for one session provisioning.
///
/// `event_id` must be the dispatcher's dedup key, so the durable unique index
/// and the in-process cache agree on what "the same delivery" means.
pub(super) fn session_provision_request(
    event_id: &str,
    connection_id: &str,
    session: &AgentSession,
) -> Result<AdapterWorkRequest, AdapterError> {
    let payload = serde_json::to_value(SessionProvisionWork {
        connection_id: connection_id.to_string(),
        session: session.clone(),
    })
    .map_err(|e| {
        AdapterError::Internal(format!("failed to serialize Linear session payload: {e}"))
    })?;
    Ok(AdapterWorkRequest {
        provider: PROVIDER,
        kind: SESSION_PROVISION,
        event_id: event_id.to_string(),
        payload,
    })
}

struct SessionProvisionHandler(AppState);

#[async_trait::async_trait]
impl AdapterWorkHandler for SessionProvisionHandler {
    async fn execute(&self, work: &AdapterWork) -> Result<(), AdapterWorkFailure> {
        let state = &self.0;
        let payload: SessionProvisionWork = serde_json::from_value(work.payload.clone())
            .map_err(|e| AdapterWorkFailure::permanent(LinearWorkError::Payload(e)))?;

        // reason: durable adapter work runs in the worker with no user
        // session; the connection is named by an already-admitted record, not
        // by caller input.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        let connection = PgAdapterConnectionRepository(state.db.clone())
            .find_by_id(&payload.connection_id, &sys_ctx)
            .await
            .map_err(|e| AdapterWorkFailure::retryable(AdapterError::Db(e)))?
            .ok_or_else(|| {
                AdapterWorkFailure::permanent(LinearWorkError::ConnectionGone(
                    payload.connection_id.clone(),
                ))
            })?;

        // Rules are re-resolved rather than snapshotted: a trigger disabled
        // between admission and execution should stop the provisioning, which
        // is what an operator disabling it means.
        let team_id = payload
            .session
            .issue
            .as_ref()
            .and_then(|issue| {
                issue
                    .team_id
                    .clone()
                    .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
            })
            .unwrap_or_default();
        let rules = super::agent_runtime::resolve_rules_for_team(state, &connection, &team_id)
            .await
            .map_err(AdapterWorkFailure::from)?;
        if rules.is_empty() {
            info!(
                session_id = %payload.session.id,
                team_id = %team_id,
                "Linear session provisioning skipped: no active rule for the team"
            );
            return Ok(());
        }

        let api = super::agent_runtime::agent_api(state, &connection.scope_value);
        super::agent_runtime::provision_selected_session_task(
            state,
            &api,
            &connection,
            &rules,
            &payload.session,
        )
        .await
        .map_err(AdapterWorkFailure::from)
    }
}

/// Log line shared by the admission site, so an operator can follow one
/// delivery from ingress into the queue.
pub(super) fn log_admission(event_id: &str, admission: crate::adapters::work::Admission) {
    match admission {
        crate::adapters::work::Admission::Admitted => {
            debug!(kind = SESSION_PROVISION, event_id, "Linear work admitted");
        }
        crate::adapters::work::Admission::Duplicate => {
            warn!(
                kind = SESSION_PROVISION,
                event_id, "Linear work already admitted for this delivery; nothing re-run"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> AgentSession {
        serde_json::from_value(serde_json::json!({
            "id": "session-A",
            "issue": {
                "id": "issue-1",
                "identifier": "ENG-7",
                "title": "Fix the thing",
                "teamId": "team-1",
                "state": {"id": "s1", "name": "Todo", "type": "unstarted"}
            },
            "comment": {"id": "c-1", "body": "@agent please"},
            "promptContext": "<context/>"
        }))
        .expect("parse session")
    }

    /// The payload is the whole of what the handler gets after a restart, and
    /// Linear's field names are renames — so the round trip has to preserve
    /// both the values and the wire spelling.
    #[test]
    fn a_session_payload_survives_the_round_trip() {
        let request =
            session_provision_request("AgentSessionEvent:created:session-A", "conn-1", &session())
                .expect("serialize");
        let back: SessionProvisionWork =
            serde_json::from_value(request.payload).expect("deserialize");

        assert_eq!(back.connection_id, "conn-1");
        assert_eq!(back.session.id, "session-A");
        let issue = back.session.issue.expect("issue survives");
        assert_eq!(issue.identifier, "ENG-7");
        assert_eq!(issue.team_id.as_deref(), Some("team-1"));
        assert_eq!(
            issue.state.expect("workflow state survives").kind,
            "unstarted"
        );
        assert_eq!(
            back.session.prompt_context.as_deref(),
            Some("<context/>"),
            "the prompt context is what the agent is given; losing it changes the task"
        );
        assert_eq!(
            back.session.comment.expect("comment survives").body,
            "@agent please"
        );
    }

    #[test]
    fn a_request_carries_the_kind_and_the_provider_key() {
        let request = session_provision_request("evt-1", "conn-1", &session()).expect("serialize");
        assert_eq!(request.provider, PROVIDER);
        assert_eq!(request.kind, "linear.session_provision");
        assert_eq!(request.event_id, "evt-1");
    }
}
