//! Linear webhook endpoint dispatcher.
//!
//! Verifies the HMAC, parses the envelope, derives the delivery identity, and
//! admits the work it carries into [`super::delivery`]. Every one of the five
//! event
//! kinds Linear sends us is admitted, so no routing, message forwarding, task
//! creation, or sandbox provisioning happens before the response.
//!
//! What is left on the response path is the part that decides whether a
//! delivery carries any work at all: the signature, the envelope, and the
//! payload-shape guards below. Those are pure reads of the request body — they
//! reach no database, no task, and no Linear API — so a redelivery repeats
//! nothing but the parse.
//!
//! Deduplication is admission's, and only admission's. A redelivery is
//! collapsed because its identity ([`dedup_key`]) is already in
//! `adapter_delivery` — on whichever replica it lands, however long after the
//! first attempt. Nothing here holds state between deliveries, which is what
//! makes a withheld acknowledgment honest: a delivery refused below leaves no
//! trace that could dismiss the sender's next attempt as a duplicate.
//!
//! The single exception is named where it lives: a newly created agent session
//! gets an ephemeral heartbeat activity posted to Linear before the `200`, so
//! its panel is alive inside Linear's 10-second window. It is a best-effort
//! notice whose failure is logged, not the work; the work behind it is admitted
//! like everything else.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::{debug, info, warn};

use crate::adapters::delivery::AdapterDeliveryRequest;
use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_webhook;
use crate::adapters::models::AdapterConnection;
use crate::state::AppState;

use super::adapter::LinearAdapter;
use super::agent_runtime;
use super::delivery::LinearWebhookError;
use super::webhook_payloads::LinearWebhookEnvelope;

#[utoipa::path(
    post,
    path = "/api/v1/adapters/linear/webhook",
    tag = "adapters",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn linear_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AdapterError> {
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("linear".into()))?;

    let linear_provider = providers
        .linear()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("linear".into()))?;

    match dispatch_webhook(&state, linear_provider, &headers, &body).await {
        Ok(event_type) => {
            record_webhook("linear", "success", "", &event_type, "", "");
        }
        // Nothing was written, so a 200 would tell Linear the delivery is ours
        // while no record of it exists — and Linear does not redeliver an
        // acknowledged event.
        Err(LinearWebhookError::NotAdmitted(e)) => {
            record_webhook("linear", "error", "not_admitted", "", "", "");
            warn!(error = %e, "Linear webhook work was not admitted; asking Linear to redeliver");
            return Err(AdapterError::from(e));
        }
        // Also not acknowledged, but for the opposite reason: admission was
        // never attempted, because the delivery carries no field that could
        // tell its work apart from the next delivery's. A `400` says so — the
        // delivery is malformed rather than the backend unwell, and no amount
        // of retrying the same body will add the missing field.
        Err(LinearWebhookError::Unidentifiable(missing)) => {
            record_webhook("linear", "error", "unidentifiable", "", "", "");
            warn!(
                missing,
                "Linear webhook refused: the delivery cannot be given an identity"
            );
            return Err(AdapterError::BadRequest(format!(
                "Linear delivery is missing {missing}"
            )));
        }
        // Everything else keeps the long-standing behaviour: log it, meter it,
        // and acknowledge.
        Err(LinearWebhookError::Ingress(e)) => {
            warn!(error = %e, "Linear webhook processing error");
            record_webhook("linear", "error", "delivery_failed", "", "", "");
        }
    }

    Ok(StatusCode::OK)
}

/// Verify the webhook, parse the envelope, and route based on `type`.
///
/// Returns the parsed `event_type` (empty string when the envelope failed to
/// parse) so the caller can label the success-path metric. Always returns
/// `Ok(_)` on benign rejections (unknown org, unsupported event, a delivery
/// carrying no work) and on a redelivery, which admission recognises as a
/// duplicate of the record it already holds.
async fn dispatch_webhook(
    state: &AppState,
    provider: &LinearAdapter,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<String, LinearWebhookError> {
    let signature = headers
        .get("linear-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let signature_present = !signature.is_empty();

    let envelope: LinearWebhookEnvelope = serde_json::from_slice(body).map_err(|e| {
        info!(
            error = %e,
            body_len = body.len(),
            signature_present,
            "Linear webhook: failed to parse envelope"
        );
        AdapterError::BadRequest(format!("Invalid Linear webhook payload: {e}"))
    })?;

    info!(
        event_type = %envelope.event_type,
        action = %envelope.action,
        org_id = envelope.organization_id.as_deref().unwrap_or(""),
        webhook_id = envelope.webhook_id.as_deref().unwrap_or(""),
        body_len = body.len(),
        signature_present,
        "Linear webhook received"
    );

    if !signature_present {
        info!("Linear webhook ignored: missing Linear-Signature header");
        return Ok(envelope.event_type.clone());
    }

    let Some(org_id) = envelope.organization_id.as_deref() else {
        info!("Linear webhook ignored: missing organizationId in envelope");
        return Ok(envelope.event_type.clone());
    };

    let connection = state
        .adapter_connections
        .find_by_external_account("linear", org_id)
        .await
        .map_err(AdapterError::Db)?;
    let Some(connection) = connection else {
        info!(
            org_id,
            "Linear webhook ignored: no integration_connection registered"
        );
        return Ok(envelope.event_type.clone());
    };

    let secret = connection
        .webhook_secret
        .as_deref()
        .or(provider.global_webhook_secret_for_dispatch());
    let Some(secret) = secret else {
        warn!(
            org_id,
            "Linear webhook rejected: no webhook secret configured on connection or env"
        );
        return Err(AdapterError::SignatureInvalid.into());
    };

    if let Err(e) = super::signature::verify_signature(secret, body, signature) {
        warn!(
            org_id,
            event_type = %envelope.event_type,
            webhook_id = envelope.webhook_id.as_deref().unwrap_or(""),
            secret_source = if connection.webhook_secret.is_some() { "connection" } else { "env" },
            signature_len = signature.len(),
            "Linear webhook rejected: signature verification failed"
        );
        return Err(e.into());
    }

    // The delivery identity, on the specific resource/event that happened and
    // NOT on Linear's `webhookId` (the stable webhook-config id, identical on
    // every delivery — see `dedup_key`). It is the only thing that collapses a
    // Linear retry: durable admission recognises it on any replica and after
    // any restart. Compute unconditionally — a resource id can exist even when
    // `webhook_id` is absent.
    let key = dedup_key(&envelope);

    let event_type = envelope.event_type.clone();
    let dispatch_result: Result<(), LinearWebhookError> = match event_type.as_str() {
        "AgentSessionEvent" => {
            handle_agent_session_event(state, &connection, &envelope, &key).await
        }
        "Issue" => admit_issue_event(state, &connection, &envelope, &key).await,
        "Comment" => admit_comment_event(state, &connection, &envelope, &key).await,
        "AppUserNotification" => {
            admit_notification_event(state, &connection, &envelope, &key).await
        }
        other => {
            let data_keys: Vec<&str> = envelope
                .data
                .as_ref()
                .and_then(|v| v.as_object())
                .map(|m| m.keys().map(String::as_str).collect())
                .unwrap_or_default();
            warn!(
                event_type = other,
                action = %envelope.action,
                org_id,
                webhook_id = envelope.webhook_id.as_deref().unwrap_or(""),
                actor_id = envelope.actor.as_ref().map_or("", |a| a.id.as_str()),
                actor_name = envelope.actor.as_ref().map_or("", |a| a.name.as_str()),
                data_keys = ?data_keys,
                body_len = body.len(),
                "Linear webhook: unsupported event type"
            );
            Ok(())
        }
    };

    dispatch_result.map(|()| event_type)
}

/// Derive a stable dedup identity for a Linear webhook delivery.
///
/// This is the only deduplication Linear has: it becomes the `event_id` of
/// every record admitted below, and the durable unique index on
/// `(adapter, kind, event_id)` is what collapses a retry — on any replica and
/// after any restart.
///
/// Linear's `webhookId` is the *webhook-configuration* id — it is identical on
/// every delivery, not unique per event — so keying on it (alone or with
/// `event_type`) would recognise distinct events as duplicates of each other
/// and drop them for the whole retention period. Instead we key on the most
/// specific stable *resource* identity available, plus the action (so e.g.
/// `created` vs `prompted` on one session don't collide).
///
/// The identity has to hold two properties at once: a genuine Linear retry
/// carries the same one, and two events a person performed separately never
/// do. A resource id alone only satisfies both when the resource can be acted
/// on once — which is true of a comment and a session creation, and false of a
/// session that can be prompted again and an issue that can be moved again.
/// Those two therefore key on something the sender varies per event.
///
/// Resource id by event type:
/// - `AgentSessionEvent.prompted` → `agentActivity.id`, the activity carrying
///   the prompt. Keying on `agentSession.id` collapses a second message to one
///   session into the first — the delivery a person is actively waiting on.
/// - `AgentSessionEvent` (other actions) → `agentSession.id`
/// - `Issue` → `data.id:webhookTimestamp`. Every update to one issue repeats
///   that issue's id, so the id alone cannot tell two state changes apart;
///   Linear resends `webhookTimestamp` unchanged on a retry, so a retry still
///   collapses.
/// - `Comment` → `data.id` (a comment id names one comment)
/// - `AppUserNotification` → `notification.id`
/// - fallback (unknown / missing) → `webhook_id:webhook_timestamp`
///   (Linear resends the same `webhookTimestamp` on genuine retries).
///
/// Key format: `"{event_type}:{action}:{resource_id}"`.
fn dedup_key(envelope: &LinearWebhookEnvelope) -> String {
    let resource_id = match envelope.event_type.as_str() {
        "AgentSessionEvent" if envelope.action == "prompted" => envelope
            .agent_activity
            .as_ref()
            .map(|activity| activity.id.clone()),
        "AgentSessionEvent" => envelope.agent_session.as_ref().map(|s| s.id.clone()),
        "Issue" => resource_uuid(envelope.data.as_ref())
            .map(|id| format!("{id}:{}", delivery_timestamp(envelope))),
        "Comment" => resource_uuid(envelope.data.as_ref()),
        "AppUserNotification" => resource_uuid(envelope.notification.as_ref()),
        _ => None,
    };

    let resource_id = resource_id.unwrap_or_else(|| {
        format!(
            "{}:{}",
            envelope.webhook_id.as_deref().unwrap_or(""),
            delivery_timestamp(envelope)
        )
    });

    format!(
        "{}:{}:{}",
        envelope.event_type, envelope.action, resource_id
    )
}

/// The `id` of whichever resource payload the envelope carried.
fn resource_uuid(payload: Option<&serde_json::Value>) -> Option<String> {
    payload
        .and_then(|v| v.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// Linear's own delivery timestamp, which it repeats unchanged on a retry.
fn delivery_timestamp(envelope: &LinearWebhookEnvelope) -> i64 {
    envelope.webhook_timestamp.unwrap_or(0)
}

/// Commit one delivery's work durably and log where it landed.
///
/// Every admission site funnels through here, so the property the acceptance
/// rests on — the response is written after the record is, or not at all — is
/// stated once. An [`AdmitError`](crate::adapters::delivery::AdmitError) becomes
/// [`LinearWebhookError::NotAdmitted`], which is the one failure the route
/// refuses to answer with a `200`.
async fn admit(
    state: &AppState,
    request: AdapterDeliveryRequest,
    event_id: &str,
) -> Result<(), LinearWebhookError> {
    let kind = request.kind;
    let admission = crate::adapters::ingress::admit_delivery(state, request).await?;
    super::delivery::log_admission(kind, event_id, admission);
    Ok(())
}

/// Admit an issue state change.
///
/// The guards are the ones this path always had, minus the rule lookup: which
/// rules are active is re-resolved when the record executes, so an operator
/// disabling one between the delivery and its execution stops the work.
async fn admit_issue_event(
    state: &AppState,
    connection: &AdapterConnection,
    envelope: &LinearWebhookEnvelope,
    event_id: &str,
) -> Result<(), LinearWebhookError> {
    // Only act on state updates.
    if envelope.action != "update" {
        return Ok(());
    }

    let Some(data) = envelope.data.as_ref() else {
        debug!("Issue.update missing data — ignoring");
        return Ok(());
    };

    let issue: super::webhook_payloads::IssuePayload = match serde_json::from_value(data.clone()) {
        Ok(i) => i,
        Err(e) => {
            debug!(error = %e, "Failed to parse IssuePayload — ignoring");
            return Ok(());
        }
    };

    if issue.resolved_team_id().is_none() {
        debug!(issue = %issue.identifier, "Issue.update missing team_id — ignoring");
        return Ok(());
    }

    // An issue's id repeats on every update, so Linear's delivery timestamp is
    // the whole of what tells two state changes on one issue apart — in the
    // delivery identity and in the forwarded message's own id alike. Without it
    // there is no identity to admit under, and substituting `0` would give
    // every timestamp-less update on this issue the same one. Refuse instead:
    // see `LinearWebhookError::Unidentifiable` for why that is the safer half
    // of the trade.
    let Some(delivered_at) = envelope.webhook_timestamp else {
        return Err(LinearWebhookError::Unidentifiable("webhookTimestamp"));
    };

    let request = super::delivery::issue_update_request(
        event_id,
        &connection.id,
        &issue,
        envelope.updated_from.as_ref(),
        delivered_at,
    )?;
    admit(state, request, event_id).await
}

/// Admit a comment on an issue.
async fn admit_comment_event(
    state: &AppState,
    connection: &AdapterConnection,
    envelope: &LinearWebhookEnvelope,
    event_id: &str,
) -> Result<(), LinearWebhookError> {
    if envelope.action != "create" {
        debug!(action = %envelope.action, "Comment event with non-create action — ignoring");
        return Ok(());
    }

    let Some(data) = envelope.data.as_ref() else {
        debug!("Comment.create missing data — ignoring");
        return Ok(());
    };

    let comment: super::webhook_payloads::CommentPayloadData =
        match serde_json::from_value(data.clone()) {
            Ok(c) => c,
            Err(e) => {
                debug!(error = %e, "Failed to parse CommentPayloadData — ignoring");
                return Ok(());
            }
        };

    let Some(issue_id) = comment.resolved_issue_id().map(str::to_string) else {
        debug!(comment_id = %comment.id, "Comment.create missing issueId — ignoring");
        return Ok(());
    };

    let request = super::delivery::comment_request(event_id, &connection.id, &issue_id, &comment)?;
    admit(state, request, event_id).await
}

/// Admit a workspace notification.
///
/// Every action is admitted, including the ones this adapter does not act on.
/// Which actions are handled is a decision, and a decision made before
/// admission leaves a whole class of Linear delivery with no durable trace of
/// having arrived — so the action is carried into the record and the worker
/// no-ops on it.
async fn admit_notification_event(
    state: &AppState,
    connection: &AdapterConnection,
    envelope: &LinearWebhookEnvelope,
    event_id: &str,
) -> Result<(), LinearWebhookError> {
    let Some(raw) = envelope.notification.as_ref() else {
        debug!("AppUserNotification missing notification payload — ignoring");
        return Ok(());
    };

    let notif: super::webhook_payloads::AppUserNotificationData =
        match serde_json::from_value(raw.clone()) {
            Ok(n) => n,
            Err(e) => {
                debug!(error = %e, "Failed to parse AppUserNotificationData — ignoring");
                return Ok(());
            }
        };

    let request =
        super::delivery::notification_request(event_id, &connection.id, &envelope.action, &notif)?;
    admit(state, request, event_id).await
}

async fn handle_agent_session_event(
    state: &AppState,
    connection: &AdapterConnection,
    envelope: &LinearWebhookEnvelope,
    event_id: &str,
) -> Result<(), LinearWebhookError> {
    let Some(session) = envelope.agent_session.as_ref() else {
        debug!("AgentSessionEvent missing agentSession payload — ignoring");
        return Ok(());
    };

    match envelope.action.as_str() {
        // The one kind that still touches Linear before answering, and it is
        // deliberate: `handle_session_created` posts an ephemeral heartbeat
        // activity so the session's panel is alive inside Linear's 10-second
        // window. The provisioning behind it is admitted, not run.
        "created" => {
            let Some(issue) = session.issue.as_ref() else {
                debug!(session_id = %session.id, "AgentSessionEvent missing issue — ignoring");
                return Ok(());
            };
            let team_id = issue
                .team_id
                .clone()
                .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
                .unwrap_or_default();
            let rules = agent_runtime::resolve_rules_for_team(state, connection, &team_id).await?;
            let api = agent_runtime::agent_api(state, &connection.scope_value);
            agent_runtime::handle_session_created(
                state, &api, connection, &rules, session, event_id,
            )
            .await?;
        }
        "prompted" => {
            let Some(activity) = envelope
                .agent_activity
                .as_ref()
                .filter(|a| !a.body.is_empty())
            else {
                debug!(session_id = %session.id, "Empty prompted body — ignoring");
                return Ok(());
            };
            let request = super::delivery::session_prompt_request(
                event_id,
                &connection.id,
                &session.id,
                &activity.id,
                &activity.body,
            )?;
            admit(state, request, event_id).await?;
        }
        other => {
            warn!(
                action = other,
                session_id = %session.id,
                "Unsupported AgentSessionEvent action"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(raw: serde_json::Value) -> LinearWebhookEnvelope {
        serde_json::from_value(raw).expect("parse envelope")
    }

    #[test]
    fn distinct_agent_sessions_sharing_a_webhook_id_get_distinct_keys() {
        // The bug: Linear reuses one stable `webhookId` across every delivery.
        // Two distinct sessions must NOT collide — previously the second was
        // dropped as a "duplicate" within the 5-minute TTL.
        let a = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "webhookId": "9628c862-stable",
            "agentSession": {"id": "session-A"}
        }));
        let b = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "webhookId": "9628c862-stable",
            "agentSession": {"id": "session-B"}
        }));
        assert_ne!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn same_session_and_action_collapses_retries() {
        let first = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "webhookId": "9628c862-stable",
            "agentSession": {"id": "session-A"}
        }));
        let retry = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "webhookId": "9628c862-stable",
            "agentSession": {"id": "session-A"}
        }));
        assert_eq!(dedup_key(&first), dedup_key(&retry));
    }

    #[test]
    fn same_session_created_vs_prompted_get_distinct_keys() {
        let created = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "agentSession": {"id": "session-A"}
        }));
        let prompted = envelope(serde_json::json!({
            "action": "prompted",
            "type": "AgentSessionEvent",
            "agentSession": {"id": "session-A"}
        }));
        assert_ne!(dedup_key(&created), dedup_key(&prompted));
    }

    /// Regression: two messages sent to one Linear agent session are two
    /// deliveries. Keying them on the session made the second indistinguishable
    /// from a retry of the first, so a person following up within the cache TTL
    /// had their message discarded.
    #[test]
    fn two_prompts_on_one_session_get_distinct_keys() {
        let first = envelope(serde_json::json!({
            "action": "prompted",
            "type": "AgentSessionEvent",
            "webhookId": "9628c862-stable",
            "webhookTimestamp": 1_700_000_000_000i64,
            "agentSession": {"id": "session-A"},
            "agentActivity": {"id": "activity-1", "type": "prompt", "body": "first"}
        }));
        let second = envelope(serde_json::json!({
            "action": "prompted",
            "type": "AgentSessionEvent",
            "webhookId": "9628c862-stable",
            "webhookTimestamp": 1_700_000_000_500i64,
            "agentSession": {"id": "session-A"},
            "agentActivity": {"id": "activity-2", "type": "prompt", "body": "second"}
        }));
        assert_ne!(dedup_key(&first), dedup_key(&second));
    }

    /// The other half of the same property: Linear retrying one prompt resends
    /// the same activity, so the retry must still collapse.
    #[test]
    fn a_retried_prompt_keeps_one_key() {
        let delivery = |timestamp: i64| {
            envelope(serde_json::json!({
                "action": "prompted",
                "type": "AgentSessionEvent",
                "webhookId": "9628c862-stable",
                "webhookTimestamp": timestamp,
                "agentSession": {"id": "session-A"},
                "agentActivity": {"id": "activity-1", "type": "prompt", "body": "first"}
            }))
        };
        // A retry may carry a later timestamp; the activity is what identifies
        // the prompt, so the key must not move with the clock.
        assert_eq!(
            dedup_key(&delivery(1_700_000_000_000)),
            dedup_key(&delivery(1_700_000_030_000))
        );
    }

    /// Regression: an issue's id repeats on every update, so two state changes
    /// keyed on it alone collided and the second was dropped.
    #[test]
    fn two_state_changes_on_one_issue_get_distinct_keys() {
        let change = |timestamp: i64, state: &str| {
            envelope(serde_json::json!({
                "action": "update",
                "type": "Issue",
                "webhookId": "9628c862-stable",
                "webhookTimestamp": timestamp,
                "data": {"id": "issue-1", "identifier": "ENG-7", "state": {"id": state}}
            }))
        };
        assert_ne!(
            dedup_key(&change(1_700_000_000_000, "in-progress")),
            dedup_key(&change(1_700_000_060_000, "in-review"))
        );
    }

    /// A Linear retry of one issue update repeats `webhookTimestamp`, so the
    /// identity holds and the redelivery is still recognised as the same one.
    #[test]
    fn a_retried_issue_update_keeps_one_key() {
        let delivery = || {
            envelope(serde_json::json!({
                "action": "update",
                "type": "Issue",
                "webhookId": "9628c862-stable",
                "webhookTimestamp": 1_700_000_000_000i64,
                "data": {"id": "issue-1", "identifier": "ENG-7", "state": {"id": "in-progress"}}
            }))
        };
        assert_eq!(dedup_key(&delivery()), dedup_key(&delivery()));
    }

    /// Two issues moving at the same instant are still two deliveries — the
    /// timestamp discriminates changes to one issue, it does not replace it.
    #[test]
    fn two_issues_changing_at_the_same_instant_get_distinct_keys() {
        let change = |issue: &str| {
            envelope(serde_json::json!({
                "action": "update",
                "type": "Issue",
                "webhookTimestamp": 1_700_000_000_000i64,
                "data": {"id": issue}
            }))
        };
        assert_ne!(dedup_key(&change("issue-1")), dedup_key(&change("issue-2")));
    }

    /// A comment id names one comment, so it needs no timestamp — and two
    /// comments on one issue must not collide the way two updates did.
    #[test]
    fn two_comments_get_distinct_keys_and_a_retry_does_not() {
        let comment = |id: &str, timestamp: i64| {
            envelope(serde_json::json!({
                "action": "create",
                "type": "Comment",
                "webhookTimestamp": timestamp,
                "data": {"id": id, "body": "hi", "issueId": "issue-1"}
            }))
        };
        assert_ne!(
            dedup_key(&comment("comment-1", 1_700_000_000_000)),
            dedup_key(&comment("comment-2", 1_700_000_000_000))
        );
        assert_eq!(
            dedup_key(&comment("comment-1", 1_700_000_000_000)),
            dedup_key(&comment("comment-1", 1_700_000_030_000)),
            "a retried comment is one delivery even if Linear retimes it"
        );
    }

    #[test]
    fn comment_and_agent_session_sharing_a_webhook_id_get_distinct_keys() {
        // Comment.create and AgentSessionEvent.created in one fan-out share a
        // `webhookId` but have different resource ids *and* event types.
        let comment = envelope(serde_json::json!({
            "action": "create",
            "type": "Comment",
            "webhookId": "shared-id",
            "data": {"id": "comment-1", "body": "hi"}
        }));
        let session = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "webhookId": "shared-id",
            "agentSession": {"id": "session-1"}
        }));
        assert_ne!(dedup_key(&comment), dedup_key(&session));
    }

    #[test]
    fn unknown_event_without_resource_id_falls_back_to_webhook_id_and_timestamp() {
        let a = envelope(serde_json::json!({
            "action": "something",
            "type": "UnknownThing",
            "webhookId": "wh-1",
            "webhookTimestamp": 1_700_000_000_000i64
        }));
        let b = envelope(serde_json::json!({
            "action": "something",
            "type": "UnknownThing",
            "webhookId": "wh-1",
            "webhookTimestamp": 1_700_000_000_001i64
        }));
        // Distinct deliveries (different timestamps) → distinct keys.
        assert_ne!(dedup_key(&a), dedup_key(&b));
        // The fallback key embeds both the webhook id and the timestamp.
        let key = dedup_key(&a);
        assert!(key.contains("wh-1"), "key should contain webhook_id: {key}");
        assert!(
            key.contains("1700000000000"),
            "key should contain webhook_timestamp: {key}"
        );
        // A genuine retry resends the same timestamp → same key (collapses).
        let retry = envelope(serde_json::json!({
            "action": "something",
            "type": "UnknownThing",
            "webhookId": "wh-1",
            "webhookTimestamp": 1_700_000_000_000i64
        }));
        assert_eq!(dedup_key(&a), dedup_key(&retry));
    }
}
