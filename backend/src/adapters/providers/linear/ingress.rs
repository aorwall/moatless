//! Linear webhook request policy and durable admission.
//!
//! The HTTP handler passes only transport facts — the signature header and the
//! body. This module owns everything from there: signature policy, connection
//! resolution, the delivery identity, the one effect that runs before the
//! acknowledgment, and durable admission.
//!
//! # Admission precedes acknowledgment
//!
//! Every one of the five event kinds Linear sends is admitted, so no routing,
//! message forwarding, task creation or sandbox provisioning happens before the
//! response. What is left on the response path is the part that decides whether
//! a delivery carries any work at all: the signature, the envelope, and
//! [`webhook::normalize`](super::webhook::normalize)'s payload-shape guards.
//! Those are pure reads of the request body — they reach no task and no Linear
//! API — so a redelivery repeats nothing but the parse.
//!
//! Deduplication is admission's, and only admission's. A redelivery is
//! collapsed because its identity ([`dedup_key`]) is already in
//! `adapter_delivery` — on whichever replica it lands, however long after the
//! first attempt. Nothing here holds state between deliveries, which is what
//! makes a withheld acknowledgment honest: a delivery refused below leaves no
//! trace that could dismiss the sender's next attempt as a duplicate.
//!
//! The single exception is [`Effect::SessionHeartbeat`](super::webhook::Effect):
//! a newly created agent session gets an ephemeral heartbeat activity posted to
//! Linear before the `200`, so its panel is alive inside Linear's 10-second
//! window. It is a best-effort notice whose failure is logged, not the work;
//! the work behind it is admitted like everything else.
//!
//! # Reachable without an HTTP request
//!
//! [`receive`] takes an `AppState`, an Adapter, a signature and a body. That is
//! the seam: every decision below the route is drivable from a test that never
//! builds a `Request`, and `backend/tests/integration/linear_ingress_test.rs` drives it.

use tracing::{info, warn};

use crate::adapters::delivery::AdapterDeliveryRequest;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, SubscriptionMatch};
use crate::state::AppState;

use super::adapter::LinearAdapter;
use super::agent_api::ActivityKind;
use super::subject::ADAPTER;
use super::webhook::{self, Effect, Plan, Requirement};
use super::webhook_payloads::LinearWebhookEnvelope;

/// Why a delivery was dropped, as the `reason` label of the `ignored` webhook
/// metric.
///
/// **This list is a public contract.** It ships with the `ignored` outcome
/// (spec Decision 4, defect D1: until then every one of these paths metered
/// `success`, so the dashboard could not tell "arrived and admitted" apart from
/// "arrived and dropped"). Adding a reason later is fine — dashboards that
/// group by reason keep working. Renaming one is not: it silently zeroes
/// whatever counted it.
pub mod reason {
    /// No `Linear-Signature` header. Nothing is verified and nothing is
    /// admitted; the header is absent rather than wrong.
    pub const NO_SIGNATURE: &str = "no_signature";
    /// The envelope names no organization, or names one with no
    /// `adapter_connections` row.
    pub const NO_CONNECTION: &str = "no_connection";
    /// An `type` this Adapter does not handle.
    pub const UNSUPPORTED_EVENT: &str = "unsupported_event";
    /// A handled `type` with an `action` this Adapter does not act on — a
    /// non-`update` Issue, a non-`create` Comment, a session neither created
    /// nor prompted.
    pub const UNSUPPORTED_ACTION: &str = "unsupported_action";
    /// The inner payload is absent, does not parse, or parses without the field
    /// the workflow keys on.
    pub const MALFORMED_PAYLOAD: &str = "malformed_payload";
    /// A session was created on a team with no active Subscription. The
    /// heartbeat is still posted; nothing durable records that Linear asked and
    /// we declined — spec Open question 2, for which this reason is the interim
    /// answer.
    pub const NO_SUBSCRIPTION: &str = "no_subscription";
    /// A prompt carrying no body, or no activity to carry one.
    pub const EMPTY_BODY: &str = "empty_body";
}

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
    NotAdmitted(#[from] crate::adapters::delivery::AdmitError),
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

/// A rejected delivery, and the event type its metric row is labelled with.
///
/// The label is carried here rather than on each variant so [`IngressError`]
/// keeps `?` ergonomics all the way down and stays variant-for-variant
/// identical to its siblings. It exists because Linear's error rows used to
/// pass `""` for `event_type` (defect D2) — the one metric that fires when
/// something breaks could not say which kind of delivery broke — and the
/// envelope is parsed before anything below it can fail, so the type is always
/// in hand except when the body is not JSON at all.
#[derive(Debug)]
pub struct Rejection {
    pub error: IngressError,
    pub event_type: String,
}

/// Verify, normalize, plan, and durably admit one Linear webhook.
pub async fn receive(
    state: &AppState,
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
                error: AdapterError::BadRequest(format!("Invalid Linear webhook payload: {e}"))
                    .into(),
                event_type: String::new(),
            });
        }
    };

    let event_type = envelope.event_type.clone();
    dispatch(state, provider, signature, body, &envelope)
        .await
        .map_err(|error| Rejection { error, event_type })
}

/// Everything `receive` does once the envelope has parsed and its event type is
/// in hand for labelling.
async fn dispatch(
    state: &AppState,
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

    let connection = state
        .adapter_connections
        .find_by_external_account("linear", org_id)
        .await
        .map_err(AdapterError::Db)?;
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
    let unmet = !requirement_met(state, &connection, requires.as_ref()).await?;

    apply(state, &connection, &effects).await;

    if unmet {
        return Ok(ignored(envelope, reason::NO_SUBSCRIPTION));
    }

    let mut kinds = Vec::with_capacity(requests.len());
    for request in requests {
        let kind = request.kind;
        admit(state, request, &event_id).await?;
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
    state: &AppState,
    connection: &AdapterConnection,
    requires: Option<&Requirement>,
) -> Result<bool, IngressError> {
    let Some(Requirement::ActiveSubscription { team_id }) = requires else {
        return Ok(true);
    };
    // Cheap guard only. The actual subscription selection — which may need a
    // `get_issue` round-trip to learn the issue's project/labels — happens
    // inside the admitted work so it never delays Linear's webhook ack.
    let rules = resolve_rules_for_team(state, connection, team_id).await?;
    if rules.is_empty() {
        warn!(
            team_id = %team_id,
            "Linear session created but no active rule found for team — emitting heartbeat only"
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
async fn apply(state: &AppState, connection: &AdapterConnection, effects: &[Effect]) {
    if effects.is_empty() {
        return;
    }
    let api = match super::auth::api_for_connection(state, connection).await {
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

/// Commit one delivery's work durably and log where it landed.
///
/// Every admission site funnels through here, so the property the acceptance
/// rests on — the response is written after the record is, or not at all — is
/// stated once. An [`AdmitError`](crate::adapters::delivery::AdmitError) becomes
/// [`IngressError::NotAdmitted`], which is the one failure the route refuses to
/// answer with a `200`.
async fn admit(
    state: &AppState,
    request: AdapterDeliveryRequest,
    event_id: &str,
) -> Result<(), IngressError> {
    let kind = request.kind;
    let admission =
        crate::adapters::ingress::admit_delivery(&state.adapter_delivery, request).await?;
    super::delivery::log_admission(kind, event_id, admission);
    Ok(())
}

/// Resolve every active subscription bound to a given Linear team. Returns
/// every subscription attached to the team — multi-listener support is preserved.
///
/// Also the worker's: `delivery.rs`'s `session_provision` and `notification`
/// executors re-resolve through here rather than snapshotting the subscriptions the
/// webhook saw, so an operator disabling a loop between admission and
/// execution stops the work.
pub(super) async fn resolve_rules_for_team(
    state: &AppState,
    connection: &AdapterConnection,
    team_id: &str,
) -> Result<Vec<SubscriptionMatch>, AdapterError> {
    let rules = state
        .loops
        .list_subscriptions_by_source_matcher(ADAPTER, team_id)
        .await?;
    Ok(rules
        .into_iter()
        .filter(|r| {
            if r.connection_id == connection.id && r.active {
                true
            } else {
                tracing::debug!(
                    subscription_id = %r.id,
                    "Linear trigger ignored (different connection or inactive)"
                );
                false
            }
        })
        .collect())
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
