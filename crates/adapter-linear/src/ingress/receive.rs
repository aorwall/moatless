//! Linear webhook request policy and durable admission.
//!
//! The HTTP handler passes only transport facts; this module owns signature
//! policy, connection resolution, the delivery identity, and admission. All five
//! event kinds are admitted before the response, so nothing on the response path
//! but the signature, the envelope and
//! [`webhook::normalize`](crate::webhook::normalize)'s shape guards — pure reads
//! of the body. Keep it that way: nothing here may hold state between
//! deliveries, or a refused delivery could dismiss the sender's next attempt as
//! a duplicate.
//!
//! [`Effect::SessionHeartbeat`](crate::webhook::Effect) is the one effect before
//! the `200`, so a new session's panel is alive inside Linear's 10-second
//! window. Best-effort; its failure is logged, and the work behind it is
//! admitted like everything else.
//!
//! [`receive`] takes its downward ports directly, so every decision below the
//! route is drivable without building a `Request`.

use super::reason;
use tracing::{info, warn};

use moatless_adapter_model::apps::AdapterAppReader;
use std::sync::Arc;

use moatless_ports::identity::AppCredentials;

use crate::subscription_rules::resolve_rules_for_team;
use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::delivery::AdapterDeliveryRequest;
use moatless_adapter_model::delivery::DeliveryAdmission;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::loops::LoopReader;
use moatless_types::adapter::AdapterConnection;

use crate::adapter::LinearAdapter;
use crate::agent_api::ActivityKind;
use crate::webhook::{self, Effect, Plan, Requirement};
use crate::webhook_payloads::LinearWebhookEnvelope;

/// Outcome surfaced so the HTTP handler can answer and emit stable webhook
/// metrics.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Admitted {
        event_type: String,
        /// The delivery kinds committed, in admission order. Carried so a test
        /// at this seam can assert what one envelope fanned into without
        /// reading `adapter_delivery` back.
        kinds: Vec<&'static str>,
    },
    Ignored {
        event_type: String,
        reason: &'static str,
    },
}

/// Why a Linear webhook did not reach a normal outcome.
///
/// The variants get different transport treatment, which is the whole reason
/// they are distinguished. [`Self::NotAdmitted`] and [`Self::Unidentifiable`]
/// both mean nothing was written, so Linear must not be told the delivery is
/// ours; [`Self::Ingress`] keeps the long-standing behaviour of a logged,
/// metered `200`.
///
/// This is Linear's copy of the three-way vocabulary `github_pr`, Slack and
/// Telegram each also declare. The hoist to one shared `adapters::ingress`
/// definition (spec Decision 2) has not landed — `adapters/ingress.rs` holds
/// only `admit_delivery` and `log_admission` — so the copy stays local, written
/// to match its siblings variant for variant so the hoist is a deletion rather
/// than a redesign.
#[derive(Debug, thiserror::Error)]
pub enum IngressError {
    /// Durable admission did not commit.
    ///
    /// Takes `#[from] AdmitError` and nothing else: a blanket
    /// `From<AdapterError>` here would let any read failure masquerade as an
    /// un-acknowledgeable one.
    #[error("Linear webhook work was not admitted")]
    NotAdmitted(#[from] moatless_adapter_model::delivery::AdmitError),
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
    /// Everything the ingress path itself rejected or failed on: an unparseable
    /// body, a missing secret, a failed signature, a database read.
    #[error(transparent)]
    Ingress(#[from] AdapterError),
}

impl From<SharedAdapterError> for IngressError {
    fn from(error: SharedAdapterError) -> Self {
        Self::Ingress(error.into())
    }
}

/// A rejected delivery, and the event type its metric row is labelled with.
///
/// The label is carried here rather than on each variant so [`IngressError`]
/// keeps `?` ergonomics all the way down and stays variant-for-variant
/// identical to its siblings. The envelope is parsed before anything below it
/// can fail, so the type is always in hand except when the body is not JSON
/// at all.
#[derive(Debug)]
pub struct Rejection {
    pub error: IngressError,
    pub event_type: String,
}

/// Verify, normalize, plan, and durably admit one Linear webhook.
#[expect(
    clippy::too_many_arguments,
    reason = "the five downward ports the ingress path reads (adapter \
              connections, loops, app tokens, adapter apps, adapter delivery) \
              inverted out of the single AppState the seam used to take; \
              provider, signature and body are the transport facts the HTTP \
              handler already passed"
)]
pub async fn receive(
    adapter_connections: &dyn ConnectionLookup,
    loops: &dyn LoopReader,
    app_tokens: &Arc<dyn AppCredentials>,
    apps: &dyn AdapterAppReader,
    adapter_delivery: &dyn DeliveryAdmission,
    provider: &LinearAdapter,
    signature: &str,
    body: &[u8],
) -> Result<Outcome, Rejection> {
    let signature_present = !signature.is_empty();

    let envelope: LinearWebhookEnvelope = match serde_json::from_slice(body) {
        Ok(envelope) => envelope,
        Err(e) => {
            info!(
                error = %e,
                body_len = body.len(),
                signature_present,
                "Linear webhook: failed to parse envelope"
            );
            // No envelope, so no event type: this is the one rejection whose
            // metric row cannot carry the label D2 added.
            return Err(Rejection {
                error: SharedAdapterError::BadRequest(format!(
                    "Invalid Linear webhook payload: {e}"
                ))
                .into(),
                event_type: String::new(),
            });
        }
    };

    let event_type = envelope.event_type.clone();
    dispatch(
        adapter_connections,
        loops,
        app_tokens,
        apps,
        adapter_delivery,
        provider,
        signature,
        body,
        &envelope,
    )
    .await
    .map_err(|error| Rejection { error, event_type })
}

/// Everything `receive` does once the envelope has parsed and its event type is
/// in hand for labelling.
#[expect(
    clippy::too_many_arguments,
    reason = "the five downward ports `receive` threads in plus the transport \
              facts and the parsed envelope; each sub-step below takes only the \
              narrow subset it needs"
)]
async fn dispatch(
    adapter_connections: &dyn ConnectionLookup,
    loops: &dyn LoopReader,
    app_tokens: &Arc<dyn AppCredentials>,
    apps: &dyn AdapterAppReader,
    adapter_delivery: &dyn DeliveryAdmission,
    provider: &LinearAdapter,
    signature: &str,
    body: &[u8],
    envelope: &LinearWebhookEnvelope,
) -> Result<Outcome, IngressError> {
    let signature_present = !signature.is_empty();

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
        return Ok(ignored(envelope, reason::NO_SIGNATURE));
    }

    let Some(org_id) = envelope.organization_id.as_deref() else {
        return Ok(ignored(envelope, reason::NO_CONNECTION));
    };

    let connection = adapter_connections
        .by_external_account("linear", org_id)
        .await
        .map_err(SharedAdapterError::Store)?;
    let Some(connection) = connection else {
        return Ok(ignored(envelope, reason::NO_CONNECTION));
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
        return Err(SharedAdapterError::SignatureInvalid.into());
    };

    if let Err(e) = crate::signature::verify_signature(secret, body, signature) {
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
    let event_id = dedup_key(envelope);

    let (requests, effects, requires) =
        match webhook::normalize(envelope, &connection.id, &event_id)? {
            Plan::Ignored { reason } => return Ok(ignored(envelope, reason)),
            Plan::Admit {
                requests,
                effects,
                requires,
            } => (requests, effects, requires),
        };

    // Settled before the effects are applied, because that is where the
    // Subscription lookup has always run: a database failure here costs Linear
    // nothing, and moving it below the heartbeat would spend an API call on a
    // delivery that is about to fail anyway.
    let unmet = !requirement_met(loops, &connection, requires.as_ref()).await?;

    if unmet {
        // A created session on a team with no active Subscription. No
        // provisioning is admitted, so nothing downstream will ever post a
        // terminal activity — close the session here instead. The heartbeat is
        // deliberately skipped: it would only keep the panel `active` before
        // the `error` that immediately follows.
        decline_sessions(app_tokens, apps, &connection, &effects).await;
        return Ok(ignored(envelope, reason::NO_SUBSCRIPTION));
    }

    apply(app_tokens, apps, &connection, &effects).await;

    let mut kinds = Vec::with_capacity(requests.len());
    for request in requests {
        let kind = request.kind;
        admit(adapter_delivery, request, &event_id).await?;
        kinds.push(kind);
    }

    Ok(Outcome::Admitted {
        event_type: envelope.event_type.clone(),
        kinds,
    })
}

/// A dropped delivery, logged once with the reason that labels its metric row.
///
/// One line for every drop, carrying what the flat dispatcher only logged on
/// the unsupported-event branch — the actor and the payload's keys are what an
/// operator needs to tell "Linear sent us something new" from "our guard is
/// wrong", and that question is asked of every reason, not just one.
fn ignored(envelope: &LinearWebhookEnvelope, reason: &'static str) -> Outcome {
    let data_keys: Vec<&str> = envelope
        .data
        .as_ref()
        .and_then(|v| v.as_object())
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    info!(
        event_type = %envelope.event_type,
        action = %envelope.action,
        webhook_id = envelope.webhook_id.as_deref().unwrap_or(""),
        actor_id = envelope.actor.as_ref().map_or("", |a| a.id.as_str()),
        actor_name = envelope.actor.as_ref().map_or("", |a| a.name.as_str()),
        data_keys = ?data_keys,
        reason,
        "Linear webhook ignored"
    );
    Outcome::Ignored {
        event_type: envelope.event_type.clone(),
        reason,
    }
}

/// Settle the one precondition [`webhook::normalize`] cannot see.
async fn requirement_met(
    loops: &dyn LoopReader,
    connection: &AdapterConnection,
    requires: Option<&Requirement>,
) -> Result<bool, IngressError> {
    let Some(Requirement::ActiveSubscription { team_id }) = requires else {
        return Ok(true);
    };
    // Cheap guard only. The actual subscription selection — which may need a
    // `get_issue` round-trip to learn the issue's project/labels — happens
    // inside the admitted work so it never delays Linear's webhook ack.
    let rules = resolve_rules_for_team(loops, connection, team_id).await?;
    if rules.is_empty() {
        warn!(
            team_id = %team_id,
            "Linear session created but no active rule found for team — closing the session"
        );
        return Ok(false);
    }
    Ok(true)
}

/// Apply the effects a plan asked for, before the acknowledgment.
///
/// Best-effort by construction: an effect is a notice about work that is about
/// to be admitted, so a failure to post it is logged and the delivery carries
/// on. Nothing here decides anything.
async fn apply(
    app_tokens: &Arc<dyn AppCredentials>,
    apps: &dyn AdapterAppReader,
    connection: &AdapterConnection,
    effects: &[Effect],
) {
    if effects.is_empty() {
        return;
    }
    let api = match crate::auth::api_for_connection(app_tokens.clone(), apps, connection).await {
        Ok(api) => api,
        Err(e) => {
            warn!(error = %e, "Failed to resolve Linear app config for webhook effect");
            return;
        }
    };
    for effect in effects {
        let Effect::SessionHeartbeat { session_id, text } = effect;
        if let Err(e) = api
            .agent_activity_create(session_id, ActivityKind::Thought, text, true)
            .await
        {
            warn!(session_id, error = %e, "Failed to emit Linear thought activity");
        }
    }
}

/// Close every session a refused plan opened, with a terminal `error` activity.
///
/// The counterpart to [`apply`] for the one admission that is dropped after its
/// panel is already open: a created session whose team has no active
/// Subscription. Linear infers session state from the last activity, so without
/// a terminal one the panel stays `active` indefinitely — this posts the `error`
/// that closes it. Best-effort, exactly like [`apply`]: a resolve or post
/// failure is logged and the drop carries on.
async fn decline_sessions(
    app_tokens: &Arc<dyn AppCredentials>,
    apps: &dyn AdapterAppReader,
    connection: &AdapterConnection,
    effects: &[Effect],
) {
    if effects.is_empty() {
        return;
    }
    let api = match crate::auth::api_for_connection(app_tokens.clone(), apps, connection).await {
        Ok(api) => api,
        Err(e) => {
            warn!(error = %e, "Failed to resolve Linear app config to decline session");
            return;
        }
    };
    for effect in effects {
        let Effect::SessionHeartbeat { session_id, .. } = effect;
        crate::sessions::decline_session(&api, session_id, crate::sessions::NO_ACTIVE_RULE_DECLINE)
            .await;
    }
}

/// Commit one delivery's work durably and log where it landed.
///
/// Every admission site funnels through here, so the property the acceptance
/// rests on — the response is written after the record is, or not at all — is
/// stated once. An [`AdmitError`](moatless_adapter_model::delivery::AdmitError) becomes
/// [`IngressError::NotAdmitted`], which is the one failure the route refuses to
/// answer with a `200`.
async fn admit(
    adapter_delivery: &dyn DeliveryAdmission,
    request: AdapterDeliveryRequest,
    event_id: &str,
) -> Result<(), IngressError> {
    let kind = request.kind;
    let admission =
        moatless_adapter_model::ingress::admit_delivery(adapter_delivery, request).await?;
    crate::delivery::log_admission(kind, event_id, admission);
    Ok(())
}

/// Derive a stable dedup identity for a Linear webhook delivery: the `event_id`
/// of every record admitted below, formatted `"{event_type}:{action}:{id}"`.
///
/// A genuine retry must carry the same key and two separately performed events
/// must never share one. A resource id alone holds both only where the resource
/// can be acted on once, so a session that can be re-prompted and an issue that
/// can be moved again key on something the sender varies per event.
///
/// Do not key on Linear's `webhookId`: it is the webhook-*configuration* id,
/// identical on every delivery, so distinct events would collapse into one and
/// be dropped for the whole retention period.
///
/// Resource id by event type:
/// - `AgentSessionEvent.prompted` → `agentActivity.id`, the activity carrying
///   the prompt. Keying on `agentSession.id` collapses a second message to one
///   session into the first — the delivery a person is actively waiting on.
/// - `AgentSessionEvent` (other actions) → `agentSession.id`
/// - `Issue` → `data.id:updatedFrom.stateId:data.state.id:data.updatedAt`.
///   Every update to one issue repeats that issue's id, so the id alone cannot
///   tell two state changes apart. Linear can emit duplicate deliveries for one
///   issue mutation with different webhook timestamps, so the issue's own
///   updatedAt is preferred; webhookTimestamp is only the fallback when
///   updatedAt is absent.
/// - `Comment` → `data.id` (a comment id names one comment)
/// - `AppUserNotification` → `notification.id`
/// - fallback (unknown / missing) → `webhook_id:webhook_timestamp`
///   (Linear resends the same `webhookTimestamp` on genuine retries).
fn dedup_key(envelope: &LinearWebhookEnvelope) -> String {
    let resource_id = match envelope.event_type.as_str() {
        "AgentSessionEvent" if envelope.action == "prompted" => envelope
            .agent_activity
            .as_ref()
            .map(|activity| activity.id.clone()),
        "AgentSessionEvent" => envelope.agent_session.as_ref().map(|s| s.id.clone()),
        "Issue" => issue_transition_resource_id(envelope),
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

fn issue_transition_resource_id(envelope: &LinearWebhookEnvelope) -> Option<String> {
    let issue_id = resource_uuid(envelope.data.as_ref())?;
    let transition_id = issue_state_transition_id(envelope)?;
    Some(format!("{issue_id}:{transition_id}"))
}

pub(crate) fn issue_state_transition_id(envelope: &LinearWebhookEnvelope) -> Option<String> {
    let data = envelope.data.as_ref()?;
    let previous_state_id = envelope
        .updated_from
        .as_ref()
        .and_then(|v| v.get("stateId"))
        .and_then(serde_json::Value::as_str)?;
    let new_state_id = data
        .get("state")
        .and_then(|v| v.get("id"))
        .and_then(serde_json::Value::as_str)?;
    let event_clock = data
        .get("updatedAt")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| envelope.webhook_timestamp.map(|ts| ts.to_string()))?;

    Some(format!("{previous_state_id}:{new_state_id}:{event_clock}"))
}

/// Linear's own delivery timestamp. For issue updates this is only a fallback:
/// Linear can send duplicate deliveries for one issue mutation with different
/// webhook timestamps.
fn delivery_timestamp(envelope: &LinearWebhookEnvelope) -> i64 {
    envelope.webhook_timestamp.unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(raw: serde_json::Value) -> LinearWebhookEnvelope {
        serde_json::from_value(raw).expect("parse envelope")
    }

    #[test]
    fn distinct_agent_sessions_sharing_a_webhook_id_get_distinct_keys() {
        // Linear reuses one stable `webhookId` across every delivery, so two
        // distinct sessions sharing it must not collide into one dedup key.
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
        let change = |updated_at: &str, state: &str| {
            envelope(serde_json::json!({
                "action": "update",
                "type": "Issue",
                "webhookId": "9628c862-stable",
                "updatedFrom": {"stateId": "todo"},
                "data": {
                    "id": "issue-1",
                    "identifier": "ENG-7",
                    "updatedAt": updated_at,
                    "state": {"id": state}
                }
            }))
        };
        assert_ne!(
            dedup_key(&change("2026-08-10T03:20:08.000Z", "in-progress")),
            dedup_key(&change("2026-08-10T03:30:59.000Z", "in-review"))
        );
    }

    /// Linear can duplicate one issue update with distinct webhook timestamps.
    /// The issue's own `updatedAt` is the stable event clock in that case.
    #[test]
    fn duplicate_issue_update_with_different_webhook_timestamps_keeps_one_key() {
        let delivery = |timestamp: i64| {
            envelope(serde_json::json!({
                "action": "update",
                "type": "Issue",
                "webhookId": "9628c862-stable",
                "webhookTimestamp": timestamp,
                "updatedFrom": {"stateId": "todo"},
                "data": {
                    "id": "issue-1",
                    "identifier": "ENG-7",
                    "updatedAt": "2026-08-10T03:20:08.000Z",
                    "state": {"id": "in-progress"}
                }
            }))
        };
        assert_eq!(
            dedup_key(&delivery(1_786_332_008_038)),
            dedup_key(&delivery(1_786_332_008_046))
        );
    }

    /// If Linear omits `updatedAt`, webhookTimestamp is still the fallback that
    /// tells two issue updates apart.
    #[test]
    fn issue_update_without_updated_at_falls_back_to_webhook_timestamp() {
        let delivery = |timestamp: i64| {
            envelope(serde_json::json!({
                "action": "update",
                "type": "Issue",
                "webhookId": "9628c862-stable",
                "webhookTimestamp": timestamp,
                "updatedFrom": {"stateId": "todo"},
                "data": {
                    "id": "issue-1",
                    "identifier": "ENG-7",
                    "state": {"id": "in-progress"}
                }
            }))
        };
        assert_ne!(
            dedup_key(&delivery(1_700_000_000_000)),
            dedup_key(&delivery(1_700_000_000_001))
        );
    }

    /// Two issues moving at the same instant are still two deliveries — the
    /// transition clock discriminates changes to one issue, it does not replace
    /// the issue id.
    #[test]
    fn two_issues_changing_at_the_same_instant_get_distinct_keys() {
        let change = |issue: &str| {
            envelope(serde_json::json!({
                "action": "update",
                "type": "Issue",
                "webhookTimestamp": 1_700_000_000_000i64,
                "updatedFrom": {"stateId": "todo"},
                "data": {
                    "id": issue,
                    "updatedAt": "2026-08-10T03:20:08.000Z",
                    "state": {"id": "in-progress"}
                }
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

    /// Every reason is a distinct string, and each is spelled exactly as the
    /// plan's Decision 4 names it. The list is a contract from the day it
    /// ships: a rename silently zeroes whatever counted it.
    #[test]
    fn the_seven_ignore_reasons_are_spelled_as_the_contract_names_them() {
        let reasons = [
            reason::NO_SIGNATURE,
            reason::NO_CONNECTION,
            reason::UNSUPPORTED_EVENT,
            reason::UNSUPPORTED_ACTION,
            reason::MALFORMED_PAYLOAD,
            reason::NO_SUBSCRIPTION,
            reason::EMPTY_BODY,
        ];
        assert_eq!(
            reasons,
            [
                "no_signature",
                "no_connection",
                "unsupported_event",
                "unsupported_action",
                "malformed_payload",
                "no_subscription",
                "empty_body",
            ]
        );
    }
}
