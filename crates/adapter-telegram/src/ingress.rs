//! Telegram update policy and durable admission.
//!
//! One `receive`, reached by both the webhook handler and the poller. What
//! differs between them is a [`Transport`] argument, not a call graph: the
//! webhook carries a secret header to verify, and long polling carries nothing.
//! A rule that is data can be tested; a rule that is a choice of call site
//! cannot.

use tracing::{debug, info};

use moatless_loops::adapters::AdapterDestinationBuilder;
use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_loops::adapters::inbound_delivery::inbound_delivery_request;
use moatless_types::adapter::InboundDeliveryPayload;
use moatless_loops::adapters::ingress::{IngressError, Outcome};
use moatless_loops::adapters::models::SubscriptionMatch;
use moatless_core::db::DbPool;

use super::adapter::TelegramAdapter;
use super::api::TelegramApi;
use super::config::TelegramAdapterConfig;
use super::webhook::{self, Effect, Normalization, NormalizeContext};

/// How the update reached us, and what that means for verification.
pub(super) enum Transport<'a> {
    /// The webhook. The header is verified against the routing Subscription.
    Webhook { secret_header: Option<&'a str> },
    /// Long polling. Telegram sends no secret here; there is nothing to verify.
    Poll,
}

/// Telegram's ignore, which carries no event type.
///
/// The Adapter has never labelled its webhook metric with one — `not_routable`
/// is the single label its dashboards query (spec Decision 9) — so the shared
/// [`Outcome`]'s `event_type` stays empty and `reason` is carried for the log
/// line the handler and the poller write.
fn ignored(reason: &'static str) -> Outcome {
    Outcome::ignored("", reason)
}

/// Verify, resolve, normalize, apply effects, and durably admit one update.
///
/// Returns only once the routing is durable (or there is none to do). Both
/// entry points must leave their acknowledgment unsent on an `Err`: the webhook
/// by answering non-2xx, the poller by holding its offset back.
pub(super) async fn receive(
    adapter: &TelegramAdapter,
    body: &[u8],
    transport: Transport<'_>,
) -> Result<Outcome, IngressError> {
    let payload: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| IngressError::Unidentifiable(format!("Invalid JSON: {e}")))?;

    // Telegram's own delivery id, and therefore the durable idempotency key: a
    // re-delivered update carries the same `update_id`, so admission recognises
    // it as the same unit of work rather than a second one.
    //
    // `update_id` is unique per *bot*, not globally, and the dedup index —
    // `(adapter, kind, event_id)` — has no connection column to scope it by.
    // That is sound only because Telegram runs a single env-backed bot
    // transport, which is the same assumption the `"default"` connection scope
    // below rests on. Give Telegram a second connection and two bots' id
    // sequences overlap: the second bot's updates would be swallowed as
    // duplicates, silently, with no error and nothing above `debug`. Whoever
    // makes Telegram multi-connection must qualify this key by connection.
    let update_id = payload
        .get("update_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| IngressError::Unidentifiable("Missing update_id".into()))?;

    // Extract chat_id from message or channel_post
    let message = payload
        .get("message")
        .or_else(|| payload.get("channel_post"));
    let chat_id = message
        .and_then(|m| m.pointer("/chat/id"))
        .and_then(serde_json::Value::as_i64);

    let Some(chat_id) = chat_id else {
        debug!("Telegram update has no chat.id, ignoring");
        return Ok(ignored("no_chat"));
    };

    let source_id = chat_id.to_string();

    let chat_type = message
        .and_then(|m| m.pointer("/chat/type"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let is_private_chat = chat_type == "private";

    // One resolution, keyed by the chat, exact match then wildcard. Telegram
    // runs a single env-backed bot transport, so the connection scope is
    // "default". The handler used to resolve a *second* time — under `"*"` for
    // a private chat — and verify that Subscription's secret, which is how an
    // update could route under a Subscription whose secret was never checked
    // (spec Decision 5).
    let Some((connection, subscription)) = moatless_loops::bindings::runtime::resolve_chat_provider_event(
        adapter
            .require_adapter_connections()
            .map_err(IngressError::Ingress)?,
        adapter.require_loops().map_err(IngressError::Ingress)?,
        "telegram",
        "default",
        &source_id,
    )
    .await
    .map_err(IngressError::Ingress)?
    else {
        info!(
            source_id,
            is_private_chat, "Telegram message ignored: no task trigger for this chat"
        );
        return Ok(ignored("no_subscription"));
    };

    // The secret belongs to the Subscription the delivery routes under, so it
    // is checked here: after resolution, before anything is normalized.
    if let Transport::Webhook { secret_header } = transport {
        verify_secret(&subscription, secret_header).map_err(IngressError::Ingress)?;
    }

    let config = moatless_adapter_model::models::subscription_context(&subscription);
    let tg_config = TelegramAdapterConfig::from_config(&config);
    let db = adapter.require_db().map_err(IngressError::Ingress)?;
    let api = adapter
        .app_api(moatless_identity::adapter_apps::app_key_from_transport_config(
            &connection.transport_config,
        ))
        .await
        .map_err(IngressError::Ingress)?;

    let normalized = webhook::normalize(
        &payload,
        &NormalizeContext {
            config: &config,
            tg_config: &tg_config,
            db,
            is_private_chat,
        },
    )
    .await
    .map_err(classify_normalization)?;

    match normalized {
        Normalization::Message { message, effects } => {
            apply_effects(&api, db, &config.id, effects)
                .await
                .map_err(IngressError::Ingress)?;
            let msg = *message;
            info!(
                source_id = %msg.source_id,
                thread_id = ?msg.thread_id,
                user_id = %msg.user_id,
                "Telegram message routed for processing"
            );
            let destination = adapter
                .binding_destination_from_existing_thread(
                    &msg.source_id,
                    &msg.message_thread_id,
                    &msg.thread_properties,
                    None,
                )
                .map_err(IngressError::Ingress)?;

            let request = inbound_delivery_request(
                super::delivery::ADAPTER,
                super::delivery::INBOUND_MESSAGE,
                update_id.to_string(),
                InboundDeliveryPayload {
                    connection_id: connection.id.clone(),
                    loop_id: subscription.id.clone(),
                    subject_kind: destination.subject_kind.to_string(),
                    subject_key: destination.subject_key,
                    destination_config: destination.destination_config,
                    // Telegram has never labelled its webhook metric with an
                    // event type; keeping it empty keeps the label set the
                    // dashboards already query.
                    event_type: String::new(),
                    message: msg,
                },
            )
            .map_err(IngressError::Ingress)?;

            // The routing runs in the Adapter delivery worker, which is also where
            // the webhook outcome metric is recorded. Failing here propagates:
            // nothing durable was written, so the caller must not acknowledge.
            let admission = moatless_loops::adapters::ingress::admit_delivery(
                adapter
                    .require_adapter_delivery()
                    .map_err(IngressError::Ingress)?,
                request,
            )
            .await?;
            debug!(
                update_id,
                admission = admission.as_str(),
                "Telegram update admitted for routing"
            );
            Ok(Outcome::Processed {
                event_type: String::new(),
            })
        }
        Normalization::Ignored { reason, effects } => {
            apply_effects(&api, db, &config.id, effects)
                .await
                .map_err(IngressError::Ingress)?;
            debug!(update_id, reason, "Telegram update ignored by provider");
            Ok(ignored(reason))
        }
    }
}

/// Normalization is the one step in [`receive`] that can reject an envelope
/// *permanently*: a chat with no id, a message with no `message_id`. No
/// re-delivery repairs those, so they belong in the class the poller drops —
/// putting them in the retryable one wedges it on that update and blocks every
/// update queued behind it.
///
/// Everything else normalization can raise is a database read that could
/// succeed next time.
fn classify_normalization(error: AdapterError) -> IngressError {
    match error {
        AdapterError::Shared(SharedAdapterError::BadRequest(message)) => {
            IngressError::Unidentifiable(message)
        }
        other => IngressError::Ingress(other),
    }
}

/// A Subscription with no `secret_token` accepts any header, including none;
/// one that has a token accepts only that exact value.
fn verify_secret(rule: &SubscriptionMatch, provided: Option<&str>) -> Result<(), AdapterError> {
    let expected = rule
        .adapter_config
        .as_ref()
        .and_then(|c| c.get("secret_token"))
        .and_then(|v| v.as_str());

    let Some(expected) = expected else {
        return Ok(());
    };

    if provided.unwrap_or("") == expected {
        Ok(())
    } else {
        Err(SharedAdapterError::SignatureInvalid.into())
    }
}

/// Do what normalization decided on but deliberately did not do.
///
/// Every Bot API call here is best-effort, exactly as the inline calls it
/// replaces were: a refused send is logged, not propagated, because the update
/// was going to be ignored either way. The pairing *write* is not best-effort —
/// a request that was never recorded is a code the admin can never approve, so
/// it fails the delivery and Telegram sends the update again.
pub(super) async fn apply_effects(
    api: &TelegramApi,
    db: &DbPool,
    loop_id: &str,
    effects: Vec<Effect>,
) -> Result<(), AdapterError> {
    for effect in effects {
        match effect {
            Effect::Reply { chat_id, text } => {
                api.send_message(&chat_id, &text, None, None).await.ok();
            }
            Effect::RequestPairing {
                chat_id,
                from_id,
                meta,
            } => {
                let from_id_str = from_id.to_string();
                let req =
                    super::pairing::upsert_pairing_request(db, loop_id, &from_id_str, Some(meta))
                        .await?;

                let message = format!(
                    "Your Telegram user ID: `{from_id}`\n\n\
                     Pairing code: `{code}`\n\n\
                     Ask your admin to approve this code.",
                    code = req.code,
                );
                if let Err(e) = api.send_message(&chat_id, &message, None, None).await {
                    // Best-effort — log but don't fail the whole flow
                    info!(from_id, error = %e, "Failed to send pairing message");
                }

                info!(
                    from_id,
                    code = %req.code,
                    loop_id = %loop_id,
                    "Pairing code sent to Telegram user"
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use moatless_types::adapter::AdapterKind;
    use super::super::test_support::test_rule;
    use super::*;

    fn rule_with(adapter_config: Option<serde_json::Value>) -> SubscriptionMatch {
        SubscriptionMatch {
            adapter_config,
            ..test_rule(AdapterKind::Telegram, "111")
        }
    }

    /// The whole secret policy: a Subscription without a token asks for
    /// nothing, one with a token accepts only that value.
    #[test]
    fn the_secret_policy_is_a_matching_header_or_no_secret_at_all() {
        let no_secret = rule_with(None);
        assert!(verify_secret(&no_secret, None).is_ok());
        assert!(verify_secret(&no_secret, Some("anything")).is_ok());

        let secret = rule_with(Some(serde_json::json!({ "secret_token": "s3cret" })));
        assert!(verify_secret(&secret, Some("s3cret")).is_ok());
        for provided in [None, Some(""), Some("wrong")] {
            assert!(
                matches!(
                    verify_secret(&secret, provided),
                    Err(AdapterError::Shared(SharedAdapterError::SignatureInvalid))
                ),
                "{provided:?} must not pass"
            );
        }
    }

    /// A malformed envelope must not reach the retryable class: the poller
    /// holds its offset back for everything there, so one `message` with no
    /// `message_id` would wedge it on that update and block every update queued
    /// behind it.
    #[test]
    fn a_malformed_envelope_is_unidentifiable_and_a_failed_read_is_not() {
        assert!(matches!(
            classify_normalization(SharedAdapterError::BadRequest("Missing message_id".into()).into()),
            IngressError::Unidentifiable(message) if message == "Missing message_id"
        ));
        assert!(matches!(
            classify_normalization(SharedAdapterError::Db(sqlx::Error::PoolTimedOut).into()),
            IngressError::Ingress(_)
        ));
    }

    /// A `secret_token` configured as the empty string is compared like any
    /// other, and an absent header reads as empty — so it accepts exactly the
    /// requests carrying no secret and refuses the ones that do. Preserved as
    /// found; it is a misconfiguration either way.
    #[test]
    fn a_blank_secret_token_accepts_only_a_blank_header() {
        let blank = rule_with(Some(serde_json::json!({ "secret_token": "" })));
        assert!(verify_secret(&blank, None).is_ok());
        assert!(verify_secret(&blank, Some("")).is_ok());
        assert!(matches!(
            verify_secret(&blank, Some("anything")),
            Err(AdapterError::Shared(SharedAdapterError::SignatureInvalid))
        ));
    }
}
