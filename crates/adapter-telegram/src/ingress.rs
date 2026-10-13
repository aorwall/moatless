//! Telegram update policy and durable admission.
//!
//! One `receive`, reached by both the webhook handler and the poller. What
//! differs between them is a [`Transport`] argument, not a call graph: the
//! webhook carries a secret header to verify, and long polling carries nothing.
//! A rule that is data can be tested; a rule that is a choice of call site
//! cannot.

use tracing::{debug, info};

use moatless_adapter_model::capabilities::AdapterDestinationBuilder;
use moatless_adapter_model::delivery::inbound_delivery_request;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::ingress::{IngressError, Outcome};
use moatless_core::db::DbPool;
use moatless_ports::identity::ProviderIdentityLink;
use moatless_types::adapter::InboundDeliveryPayload;
use moatless_types::loops::SubscriptionMatch;

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

/// Telegram's ignore outcome, which carries no event type.
///
/// `event_type` stays empty; `reason` is carried for the log line the handler
/// and poller write.
fn ignored(reason: &'static str) -> Outcome {
    Outcome::ignored("", reason)
}

/// Verify, resolve, normalize, apply effects, and durably admit one update.
///
/// On `Err`, the caller must leave its acknowledgment unsent: non-2xx for the
/// webhook, held offset for the poller.
pub(super) async fn receive(
    adapter: &TelegramAdapter,
    users: &dyn ProviderIdentityLink,
    body: &[u8],
    transport: Transport<'_>,
) -> Result<Outcome, IngressError> {
    let payload: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| IngressError::Unidentifiable(format!("Invalid JSON: {e}")))?;

    // `update_id` is Telegram's durable idempotency key, unique per *bot* only.
    // The dedup key (`adapter, kind, event_id`) has no connection column, sound
    // only while Telegram runs one env-backed bot. A second connection would
    // overlap id sequences and silently swallow its updates as duplicates —
    // qualify this key by connection then.
    let update_id = payload
        .get("update_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| IngressError::Unidentifiable("Missing update_id".into()))?;

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
    // "default". No second resolution under `"*"` for a private chat: verifying
    // that Subscription's secret would let an update route under a Subscription
    // whose secret was never checked (spec Decision 5).
    let Some((connection, subscription)) =
        moatless_adapter_model::connections::resolve_chat_provider_event(
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

    let config = moatless_adapter_model::loops::subscription_context(&subscription);
    let tg_config = TelegramAdapterConfig::from_config(&config);
    let db = adapter.require_db().map_err(IngressError::Ingress)?;
    let api = adapter
        .app_api(moatless_adapter_model::apps::app_key_from_transport_config(
            &connection.transport_config,
        ))
        .await
        .map_err(IngressError::Ingress)?;

    let normalized = webhook::normalize(
        &payload,
        &NormalizeContext {
            config: &config,
            tg_config: &tg_config,
            users,
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
            let admission = moatless_adapter_model::ingress::admit_delivery(
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

/// Classifies a normalization failure as permanent or retryable.
///
/// Permanent (a chat or message with no id) must not enter the retryable class:
/// that wedges the poller on it.
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
/// Bot API sends here are best-effort (failures logged, not propagated); the
/// pairing write is not — its failure fails the delivery so Telegram retries.
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
    use super::super::test_support::test_rule;
    use super::*;
    use moatless_types::adapter::AdapterKind;

    fn rule_with(adapter_config: Option<serde_json::Value>) -> SubscriptionMatch {
        SubscriptionMatch {
            name: "test-loop".to_string(),
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
    /// holds its offset back there, so a `message` with no `message_id` would
    /// wedge it.
    #[test]
    fn a_malformed_envelope_is_unidentifiable_and_a_failed_read_is_not() {
        assert!(matches!(
            classify_normalization(SharedAdapterError::BadRequest("Missing message_id".into()).into()),
            IngressError::Unidentifiable(message) if message == "Missing message_id"
        ));
        assert!(matches!(
            classify_normalization(
                SharedAdapterError::Store(moatless_types::store::StoreError::with_source(
                    "read the connection",
                    sqlx::Error::PoolTimedOut,
                ))
                .into()
            ),
            IngressError::Ingress(_)
        ));
    }

    /// A blank `secret_token` is compared like any other value, and an absent
    /// header reads as empty — so it accepts only requests carrying no secret.
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
