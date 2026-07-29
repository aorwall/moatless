//! Durable admission for every Linear webhook event kind.
//!
//! Everything a Linear delivery triggers used to run on the response path:
//! provisioning the task behind a new agent session was `tokio::spawn`ed, and
//! the other four kinds — a follow-up prompt on an existing session, an issue
//! state change, a comment, and a workspace notification — ran inline before
//! the `200`. Either shape makes the acknowledgment a lie: Linear considers an
//! acknowledged delivery finished and does not send it again, so a deploy, an
//! out-of-memory kill, or a panic lost the work permanently and with no trace.
//!
//! All five are now admitted into [`crate::adapters::delivery`] before the response
//! is returned. The admission contract lives in that module and is not restated
//! here; what this module owns is the Linear-specific half: the kinds, the
//! payloads, and the idempotency key.
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
//! # One kind per unit of work
//!
//! A Linear delivery carries one event type, so it becomes one record — but the
//! kinds are still distinct rather than one shared `linear.webhook`, because
//! dedup and the retry budget are both per `(adapter, kind, event_id)`. Folding
//! them together would let a permanent failure in one spend the attempts of the
//! others, which is the rule the GitHub kinds already follow.
//!
//! # What still runs on the response path
//!
//! Signature verification, envelope parsing, and the payload-shape guards that
//! decide whether a delivery carries any work at all. Everything with an effect
//! — resolving rules, reading bindings, calling Linear's API, forwarding a
//! message, creating or closing a task — happens in the worker. Anything else
//! appearing between the signature check and the response is a defect.
//!
//! # The only deduplication Linear has
//!
//! The unique index on `(adapter, kind, event_id)` — nothing else. The
//! adapter used to also hold a 5-minute in-process cache, marked before the
//! dispatch ran and evicted again whenever admission failed. Every kind now has
//! a durable identity that survives a restart, a second replica, and the
//! expiry, which is strictly more than the cache ever provided; it and the
//! eviction compensating for it are gone. Nothing between the signature check
//! and the response may swallow a delivery the sender is about to be asked to
//! send again.
//!
//! # Execution deadline
//!
//! Provisioning is the slow work this seam exists for, and
//! [`AdapterDeliveryPolicy::execution_timeout`](crate::adapters::delivery::AdapterDeliveryPolicy)
//! is 2 minutes. If it is exceeded the attempt is abandoned and retried, and
//! the retry converges on the binding guard in `provision_session_task` rather
//! than creating a second task. The uncovered window is a timeout landing
//! between task creation and that binding write; the spawned path lost the
//! session outright in the same window.
//!
//! The other four kinds are at-least-once too, and each converges on an
//! identity it does not invent: a prompt on `linear:prompt:{activity_id}`, a
//! comment on `linear:comment:{comment_id}`, and an issue state change on
//! `linear:issue-state:{issue_id}:{delivered_at}` — which is why the delivery
//! timestamp is carried in the payload rather than re-read from a clock the
//! worker would see differently on every attempt.

use moatless_backend_core::scope::ScopeContext;
use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::delivery::{
    AdapterDelivery, AdapterDeliveryFailure, AdapterDeliveryHandler, AdapterDeliveryHandlers,
    AdapterDeliveryRequest, AdmitError,
};
use crate::adapters::error::AdapterError;
use crate::adapters::models::AdapterConnection;
use crate::state::AppState;

use super::webhook_payloads::{
    AgentSession, AppUserNotificationData, CommentPayloadData, IssuePayload,
};

/// Adapter name on every record this module admits.
pub const ADAPTER: &str = "linear";

/// Provision the task behind a newly created Linear agent session.
pub const SESSION_PROVISION: &str = "linear.session_provision";

/// Forward a follow-up prompt sent to an existing Linear agent session.
pub const SESSION_PROMPT: &str = "linear.session_prompt";

/// Act on an issue's workflow state change: notify, close, and status-entry
/// task creation.
pub const ISSUE_UPDATE: &str = "linear.issue_update";

/// Forward a comment on a bound issue to the tasks listening to it.
pub const COMMENT: &str = "linear.comment";

/// Act on a workspace notification from Linear's OAuth-app inbox stream.
pub const NOTIFICATION: &str = "linear.notification";

/// Register the Linear delivery handlers on the process-wide registry.
///
/// One call from the composition root; the worker then claims exactly these
/// kinds. A build without this call leaves Linear records for a replica that
/// has it, rather than failing them.
pub fn register(handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
    handlers.register(
        SESSION_PROVISION,
        Arc::new(SessionProvisionHandler(state.clone())),
    );
    handlers.register(
        SESSION_PROMPT,
        Arc::new(SessionPromptHandler(state.clone())),
    );
    handlers.register(ISSUE_UPDATE, Arc::new(IssueUpdateHandler(state.clone())));
    handlers.register(COMMENT, Arc::new(CommentHandler(state.clone())));
    handlers.register(NOTIFICATION, Arc::new(NotificationHandler(state.clone())));
}

/// Why a Linear webhook did not reach a normal outcome.
///
/// The variants get opposite transport treatment. [`Self::NotAdmitted`] and
/// [`Self::Unidentifiable`] both mean nothing was written, so Linear must not
/// be told the delivery is ours; [`Self::Ingress`] keeps the long-standing
/// behaviour of a logged, metered `200`.
#[derive(Debug, thiserror::Error)]
pub enum LinearWebhookError {
    /// Durable admission did not commit.
    #[error("Linear webhook work was not admitted")]
    NotAdmitted(#[from] AdmitError),
    /// The delivery carries work, but not the field that would tell that work
    /// apart from the next delivery like it.
    ///
    /// Admitting anyway is the one outcome that is worse than refusing.
    /// A colliding identity does not lose *this* delivery, it loses the *next*
    /// one — silently, cluster-wide, and for the whole retention period,
    /// because the durable unique index would recognise a genuinely different
    /// event as a duplicate of this one. Refusing keeps the loss where an
    /// operator can see it.
    #[error("Linear delivery is missing {0}, so its work cannot be told apart from the next")]
    Unidentifiable(&'static str),
    /// Everything the dispatch path itself rejected or failed on.
    #[error(transparent)]
    Ingress(#[from] AdapterError),
}

/// Everything the provisioning handler needs to redo the work from scratch.
///
/// The connection is referenced by id rather than snapshotted, so a webhook
/// secret never lands in a table retained for weeks and a record executed
/// minutes later sees the connection as it is. The session, by contrast, is
/// carried verbatim: it is the sender's own event payload and cannot be
/// re-fetched from anywhere once the request body is gone.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SessionProvisionDelivery {
    pub(super) connection_id: String,
    pub(super) session: AgentSession,
}

/// Everything the prompt handler needs to forward one follow-up prompt.
///
/// `activity_id` is what the forwarded message is identified by downstream
/// (`linear:prompt:{activity_id}`), so it has to survive the restart with the
/// body: rebuilding it from anything the worker observes — a clock, a fresh
/// uuid — would give every attempt its own identity and double-deliver the
/// prompt to the Agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SessionPromptDelivery {
    pub(super) connection_id: String,
    pub(super) session_id: String,
    pub(super) activity_id: String,
    pub(super) body: String,
}

/// Everything the issue handler needs to act on one workflow state change.
///
/// `delivered_at` is Linear's own `webhookTimestamp`. It is half of both the
/// delivery identity and the forwarded message's
/// `linear:issue-state:{issue}:{delivered_at}` id, and Linear repeats it
/// unchanged on a retry — so it is carried rather than re-derived, which is
/// what keeps a retried attempt from looking like a new state change.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct IssueUpdateDelivery {
    pub(super) connection_id: String,
    pub(super) issue: IssuePayload,
    /// Linear's `updatedFrom` diff, which names the state the issue left. A
    /// state change cannot be recognised without it.
    #[serde(default)]
    pub(super) updated_from: Option<serde_json::Value>,
    pub(super) delivered_at: i64,
}

/// Everything the comment handler needs to forward one comment.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CommentDelivery {
    pub(super) connection_id: String,
    pub(super) issue_id: String,
    pub(super) comment: CommentPayloadData,
}

/// Everything the notification handler needs to act on one workspace
/// notification.
///
/// `action` is carried rather than filtered on at the route, because whether an
/// action is one this adapter acts on is a decision, and every decision belongs
/// behind admission. Filtering at the route would leave a whole class of Linear
/// delivery with no durable record of having arrived — which is the question an
/// operator asks first and the one this seam exists to answer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct NotificationDelivery {
    pub(super) connection_id: String,
    pub(super) action: String,
    pub(super) notification: AppUserNotificationData,
}

/// Failures this module raises on its own behalf.
#[derive(Debug, thiserror::Error)]
enum LinearDeliveryError {
    #[error("adapter connection {0} no longer exists")]
    ConnectionGone(String),
    #[error("Adapter delivery payload does not match its kind")]
    Payload(#[source] serde_json::Error),
}

/// Build the admission request for one Linear delivery.
///
/// `event_id` must be the dispatcher's dedup key, so that one delivery has one
/// identity no matter which kind it fans into. Fails only if the payload cannot
/// be serialized, which for these types means a `serde_json::Value` carrying a
/// non-string map key — impossible for a value parsed from a webhook body, but
/// not worth an `unwrap`.
fn request(
    kind: &'static str,
    event_id: &str,
    payload: &impl serde::Serialize,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    let payload = serde_json::to_value(payload).map_err(|e| {
        AdapterError::Internal(format!("failed to serialize {kind} work payload: {e}"))
    })?;
    Ok(AdapterDeliveryRequest {
        adapter: ADAPTER,
        kind,
        event_id: event_id.to_string(),
        payload,
    })
}

/// Build the admission request for one session provisioning.
pub(super) fn session_provision_request(
    event_id: &str,
    connection_id: &str,
    session: &AgentSession,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    request(
        SESSION_PROVISION,
        event_id,
        &SessionProvisionDelivery {
            connection_id: connection_id.to_string(),
            session: session.clone(),
        },
    )
}

/// Build the admission request for one follow-up prompt.
pub(super) fn session_prompt_request(
    event_id: &str,
    connection_id: &str,
    session_id: &str,
    activity_id: &str,
    body: &str,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    request(
        SESSION_PROMPT,
        event_id,
        &SessionPromptDelivery {
            connection_id: connection_id.to_string(),
            session_id: session_id.to_string(),
            activity_id: activity_id.to_string(),
            body: body.to_string(),
        },
    )
}

/// Build the admission request for one issue state change.
pub(super) fn issue_update_request(
    event_id: &str,
    connection_id: &str,
    issue: &IssuePayload,
    updated_from: Option<&serde_json::Value>,
    delivered_at: i64,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    request(
        ISSUE_UPDATE,
        event_id,
        &IssueUpdateDelivery {
            connection_id: connection_id.to_string(),
            issue: issue.clone(),
            updated_from: updated_from.cloned(),
            delivered_at,
        },
    )
}

/// Build the admission request for one comment.
pub(super) fn comment_request(
    event_id: &str,
    connection_id: &str,
    issue_id: &str,
    comment: &CommentPayloadData,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    request(
        COMMENT,
        event_id,
        &CommentDelivery {
            connection_id: connection_id.to_string(),
            issue_id: issue_id.to_string(),
            comment: comment.clone(),
        },
    )
}

/// Build the admission request for one workspace notification.
pub(super) fn notification_request(
    event_id: &str,
    connection_id: &str,
    action: &str,
    notification: &AppUserNotificationData,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    request(
        NOTIFICATION,
        event_id,
        &NotificationDelivery {
            connection_id: connection_id.to_string(),
            action: action.to_string(),
            notification: notification.clone(),
        },
    )
}

/// Read a record's payload back, or end the record.
///
/// A payload that does not deserialize was written by a build that is gone, or
/// is corrupt; no retry can fix either.
fn parse<T: serde::de::DeserializeOwned>(
    delivery: &AdapterDelivery,
) -> Result<T, AdapterDeliveryFailure> {
    serde_json::from_value(delivery.payload.clone())
        .map_err(|e| AdapterDeliveryFailure::permanent(LinearDeliveryError::Payload(e)))
}

/// Load the connection a record was admitted for.
///
/// Ids, not snapshots: re-resolving keeps the webhook secret out of a table
/// retained for weeks, and a record executed minutes later sees the connection
/// as it is. A connection deleted in between is permanent — the tenant is gone
/// and no retry brings it back.
async fn resolve_connection(
    state: &AppState,
    connection_id: &str,
) -> Result<AdapterConnection, AdapterDeliveryFailure> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a durable Adapter delivery runs in the worker with no user session; the connection \
                  is named by an already-admitted record, not by caller input"
    )]
    let sys_ctx = ScopeContext::system();
    PgAdapterConnectionRepository(state.db.clone())
        .find_by_id(connection_id, &sys_ctx)
        .await
        .map_err(|e| AdapterDeliveryFailure::retryable(AdapterError::Db(e)))?
        .ok_or_else(|| {
            AdapterDeliveryFailure::permanent(LinearDeliveryError::ConnectionGone(
                connection_id.to_string(),
            ))
        })
}

struct SessionProvisionHandler(AppState);

#[async_trait::async_trait]
impl AdapterDeliveryHandler for SessionProvisionHandler {
    async fn execute(&self, delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
        let state = &self.0;
        let payload: SessionProvisionDelivery = parse(delivery)?;
        let connection = resolve_connection(state, &payload.connection_id).await?;

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
            .map_err(AdapterDeliveryFailure::from)?;
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
        .map_err(AdapterDeliveryFailure::from)
    }
}

struct SessionPromptHandler(AppState);

#[async_trait::async_trait]
impl AdapterDeliveryHandler for SessionPromptHandler {
    async fn execute(&self, delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
        let state = &self.0;
        let payload: SessionPromptDelivery = parse(delivery)?;
        let connection = resolve_connection(state, &payload.connection_id).await?;
        let api = super::agent_runtime::agent_api(state, &connection.scope_value);
        super::agent_runtime::handle_session_prompted(
            state,
            &api,
            &connection,
            &payload.session_id,
            &payload.activity_id,
            &payload.body,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)
    }
}

struct IssueUpdateHandler(AppState);

#[async_trait::async_trait]
impl AdapterDeliveryHandler for IssueUpdateHandler {
    async fn execute(&self, delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
        let state = &self.0;
        let payload: IssueUpdateDelivery = parse(delivery)?;
        let connection = resolve_connection(state, &payload.connection_id).await?;

        // Re-resolved rather than snapshotted, for the same reason the
        // provisioning handler re-resolves: a rule disabled between admission
        // and execution should stop the work, which is what disabling it means.
        let team_id = payload.issue.resolved_team_id().unwrap_or_default();
        let rules = super::agent_runtime::resolve_rules_for_team(state, &connection, team_id)
            .await
            .map_err(AdapterDeliveryFailure::from)?;
        if rules.is_empty() {
            debug!(
                issue = %payload.issue.identifier,
                team_id,
                "Linear issue update skipped: no active rule for the team"
            );
            return Ok(());
        }

        let api = super::agent_runtime::agent_api(state, &connection.scope_value);
        super::agent_runtime::handle_issue_update(
            state,
            &api,
            &connection,
            &rules,
            payload.updated_from.as_ref(),
            &payload.issue,
            payload.delivered_at,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)
    }
}

struct CommentHandler(AppState);

#[async_trait::async_trait]
impl AdapterDeliveryHandler for CommentHandler {
    async fn execute(&self, delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
        let state = &self.0;
        let payload: CommentDelivery = parse(delivery)?;
        let connection = resolve_connection(state, &payload.connection_id).await?;
        let api = super::agent_runtime::agent_api(state, &connection.scope_value);
        super::agent_runtime::handle_comment_created(
            state,
            &api,
            &connection,
            &payload.issue_id,
            &payload.comment,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)
    }
}

struct NotificationHandler(AppState);

#[async_trait::async_trait]
impl AdapterDeliveryHandler for NotificationHandler {
    async fn execute(&self, delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
        let state = &self.0;
        let payload: NotificationDelivery = parse(delivery)?;
        let connection = resolve_connection(state, &payload.connection_id).await?;

        // An action this adapter does not act on is a *successful* no-op, not a
        // failure: the delivery arrived, it was recorded, and there was nothing
        // to do with it. Ending the record any other way would put a delivery
        // Linear sends routinely into an operator's failed-record queue.
        if payload.action != "issueStatusChanged" {
            warn!(
                action = %payload.action,
                org_id = connection.scope_value.as_str(),
                notification_id = payload.notification.id.as_deref().unwrap_or(""),
                notification_kind = payload.notification.kind.as_str(),
                "Linear AppUserNotification: action not handled, nothing to do"
            );
            return Ok(());
        }

        let api = super::agent_runtime::agent_api(state, &connection.scope_value);
        super::agent_runtime::handle_issue_status_notification(
            state,
            &api,
            &connection,
            &payload.notification,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)
    }
}

/// Log line shared by every admission site, so an operator can follow one
/// delivery from ingress into the queue.
pub(super) fn log_admission(
    kind: &'static str,
    event_id: &str,
    admission: crate::adapters::delivery::Admission,
) {
    match admission {
        crate::adapters::delivery::Admission::Admitted => {
            debug!(kind, event_id, "Linear delivery admitted");
        }
        crate::adapters::delivery::Admission::Duplicate => {
            warn!(
                kind,
                event_id, "Linear delivery already admitted; nothing re-run"
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
        let back: SessionProvisionDelivery =
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
    fn a_request_carries_the_kind_and_the_senders_key() {
        let request = session_provision_request("evt-1", "conn-1", &session()).expect("serialize");
        assert_eq!(request.adapter, ADAPTER);
        assert_eq!(request.kind, "linear.session_provision");
        assert_eq!(request.event_id, "evt-1");
    }

    /// Every kind is its own record, so a permanent failure in one cannot spend
    /// the retry budget of another derived from the same delivery.
    #[test]
    fn the_kinds_are_distinct_and_namespaced() {
        let kinds = [
            SESSION_PROVISION,
            SESSION_PROMPT,
            ISSUE_UPDATE,
            COMMENT,
            NOTIFICATION,
        ];
        let mut unique = kinds.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), kinds.len(), "two kinds share a name");
        assert!(kinds.iter().all(|k| k.starts_with("linear.")));
    }

    /// One delivery has one identity. Two kinds admitted under it are two
    /// records because the *kind* differs, not because the key does — asserting
    /// that here keeps a future "make the key unique per kind" change from
    /// silently breaking the dedup the identity provides.
    #[test]
    fn every_kind_admits_under_the_dispatchers_delivery_identity() {
        let event_id = "AgentSessionEvent:prompted:activity-1";
        let prompt = session_prompt_request(event_id, "conn-1", "session-A", "activity-1", "hi")
            .expect("serialize");
        let provision =
            session_provision_request(event_id, "conn-1", &session()).expect("serialize");

        assert_eq!(prompt.event_id, event_id);
        assert_eq!(provision.event_id, event_id);
        assert_ne!(prompt.kind, provision.kind);
    }

    /// The activity id is what the forwarded prompt is identified by
    /// downstream, so losing it in the round trip would double-deliver the
    /// prompt on the first retry.
    #[test]
    fn a_prompt_payload_survives_the_round_trip() {
        let request = session_prompt_request(
            "AgentSessionEvent:prompted:activity-9",
            "conn-1",
            "session-A",
            "activity-9",
            "please look again",
        )
        .expect("serialize");
        assert_eq!(request.kind, "linear.session_prompt");

        let back: SessionPromptDelivery =
            serde_json::from_value(request.payload).expect("deserialize");
        assert_eq!(back.connection_id, "conn-1");
        assert_eq!(back.session_id, "session-A");
        assert_eq!(back.activity_id, "activity-9");
        assert_eq!(back.body, "please look again");
    }

    /// Linear's delivery timestamp is half of the forwarded message's identity,
    /// so it has to come back off the record exactly — an attempt that lost it
    /// would look like a different state change to everything downstream.
    #[test]
    fn an_issue_payload_survives_the_round_trip_with_its_delivery_timestamp() {
        let issue: IssuePayload = serde_json::from_value(serde_json::json!({
            "id": "issue-1",
            "identifier": "ENG-7",
            "title": "Fix the thing",
            "teamId": "team-1",
            "state": {"id": "s2", "name": "In Progress", "type": "started"}
        }))
        .expect("parse issue");
        let updated_from = serde_json::json!({"stateId": "s1"});

        let request = issue_update_request(
            "Issue:update:issue-1:1700000000000",
            "conn-1",
            &issue,
            Some(&updated_from),
            1_700_000_000_000,
        )
        .expect("serialize");
        assert_eq!(request.kind, "linear.issue_update");

        let back: IssueUpdateDelivery =
            serde_json::from_value(request.payload).expect("deserialize");
        assert_eq!(back.connection_id, "conn-1");
        assert_eq!(back.issue.id, "issue-1");
        assert_eq!(back.issue.resolved_team_id(), Some("team-1"));
        assert_eq!(back.issue.state_kind(), "started");
        assert_eq!(
            back.updated_from
                .as_ref()
                .and_then(|v| v.get("stateId"))
                .and_then(serde_json::Value::as_str),
            Some("s1"),
            "without the diff the handler cannot tell a state change happened"
        );
        assert_eq!(back.delivered_at, 1_700_000_000_000);
    }

    #[test]
    fn a_comment_payload_survives_the_round_trip() {
        let comment: CommentPayloadData = serde_json::from_value(serde_json::json!({
            "id": "comment-1",
            "body": "have a look",
            "issueId": "issue-1",
            "parentId": "comment-0",
            "user": {"id": "user-1", "name": "Alice"}
        }))
        .expect("parse comment");

        let request = comment_request("Comment:create:comment-1", "conn-1", "issue-1", &comment)
            .expect("serialize");
        assert_eq!(request.kind, "linear.comment");

        let back: CommentDelivery = serde_json::from_value(request.payload).expect("deserialize");
        assert_eq!(back.issue_id, "issue-1");
        assert_eq!(back.comment.id, "comment-1");
        assert_eq!(back.comment.body, "have a look");
        assert_eq!(
            back.comment.parent_id.as_deref(),
            Some("comment-0"),
            "the parent decides which task a threaded reply belongs to"
        );
        assert_eq!(
            back.comment.user.as_ref().map(|u| u.id.as_str()),
            Some("user-1"),
            "the author is what the echo filter reads"
        );
    }

    #[test]
    fn a_notification_payload_survives_the_round_trip() {
        let notification: AppUserNotificationData = serde_json::from_value(serde_json::json!({
            "id": "notif-1",
            "type": "issueStatusChanged",
            "issueId": "issue-1",
            "issue": {
                "id": "issue-1",
                "identifier": "ENG-7",
                "title": "Fix the thing",
                "teamId": "team-1",
                "state": {"id": "s3", "name": "Done", "type": "completed"}
            }
        }))
        .expect("parse notification");

        let request = notification_request(
            "AppUserNotification:issueStatusChanged:notif-1",
            "conn-1",
            "issueStatusChanged",
            &notification,
        )
        .expect("serialize");
        assert_eq!(request.kind, "linear.notification");

        let back: NotificationDelivery =
            serde_json::from_value(request.payload).expect("deserialize");
        assert_eq!(
            back.action, "issueStatusChanged",
            "the worker decides whether an action is handled, so it has to arrive"
        );
        assert_eq!(back.notification.kind, "issueStatusChanged");
        let issue = back.notification.resolved_issue().expect("issue survives");
        assert_eq!(issue.state_kind(), "completed");
        assert_eq!(issue.resolved_team_id(), Some("team-1"));
    }

    /// An action this adapter does not act on is still admitted, and the record
    /// still names it. Filtering at the route would leave the delivery with no
    /// durable trace at all.
    #[test]
    fn an_unhandled_notification_action_is_still_carried_into_the_record() {
        let notification: AppUserNotificationData = serde_json::from_value(serde_json::json!({
            "id": "notif-2",
            "type": "issueMention",
            "issueId": "issue-1",
        }))
        .expect("parse notification");

        let request = notification_request(
            "AppUserNotification:issueMention:notif-2",
            "conn-1",
            "issueMention",
            &notification,
        )
        .expect("serialize");

        let back: NotificationDelivery =
            serde_json::from_value(request.payload).expect("deserialize");
        assert_eq!(back.action, "issueMention");
        assert_eq!(back.notification.id.as_deref(), Some("notif-2"));
    }
}
