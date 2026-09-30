//! Verify webhook inputs and admit durable inbound work.
use crate::delivery::{ADAPTER, INBOUND_MESSAGE, SUBJECT_KIND};
use moatless_adapter_model::delivery::{DeliveryAdmission, inbound_delivery_request};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::ingress::admit_delivery;
use moatless_adapter_model::loops::LoopReader;
use moatless_api_schemas::webhook::{WebhookAccepted, WebhookPayload};
use moatless_types::adapter::{AdapterKind, Audience, InboundDeliveryPayload, InboundMessage};
use moatless_types::loops::LoopContext;
use tracing::{debug, info};
/// One inbound delivery, verified and parsed, ready to be admitted.
pub(crate) struct WebhookDelivery {
    /// The identity this delivery is admitted under.
    pub(crate) delivery_id: String,
    /// The message the admitted work will route.
    pub(crate) message: InboundMessage,
}

/// Parse an inbound webhook request into a [`WebhookDelivery`].
///
/// Verifies the HMAC signature if `webhook_secret` is configured; the delivery
/// identity is read from inside the verified body, so a signed request signs
/// its identifier too.
pub(crate) fn parse_inbound(
    config: &LoopContext,
    body: &[u8],
    signature: Option<&str>,
) -> Result<WebhookDelivery, AdapterError> {
    // 1. Verify signature if webhook_secret is configured
    let webhook_secret = config
        .adapter_config
        .as_ref()
        .and_then(|c| c.get("webhook_secret"))
        .and_then(|v| v.as_str());

    if let Some(secret) = webhook_secret {
        let sig = signature.ok_or(SharedAdapterError::SignatureInvalid)?;
        super::signature::verify_signature(secret, body, sig)?;
    }

    // 2. Parse payload
    let payload: WebhookPayload = serde_json::from_slice(body)
        .map_err(|e| SharedAdapterError::BadRequest(format!("Invalid JSON: {e}")))?;

    if payload.text.trim().is_empty() {
        return Err(SharedAdapterError::BadRequest("Missing required field: text".into()).into());
    }

    // 3. Determine user_id
    let user_id = payload
        .user_id
        .or_else(|| {
            config
                .adapter_config
                .as_ref()
                .and_then(|c| c.get("user_id"))
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| config.effective_user_id().to_string());

    // 4. Build message thread ID (use provided thread_id or generate one)
    let message_thread_id = payload
        .thread_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // 5. Build thread properties
    let thread_properties = serde_json::json!({
        "loop_id": config.id,
        "metadata": payload.metadata,
    });

    // 6. Settle the delivery identity. A caller that supplied none gets a
    // fresh one, which is what makes every delivery admissible without
    // forcing an arbitrary client to invent an identifier scheme it has no
    // retry contract for. See `WebhookPayload::delivery_id`.
    let delivery_id = payload
        .delivery_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // 7. The identity routing sees. Settled here, once, so every execution
    // of the admitted record carries the same one — which is what makes a
    // re-execution after a lost process converge instead of delivering the
    // message twice.
    let external_event_id = super::external_id::delivery(&delivery_id);

    Ok(WebhookDelivery {
        delivery_id,
        message: InboundMessage {
            adapter_kind: AdapterKind::Webhook,
            source_id: config.source_id.clone(),
            thread_id: payload.thread_id,
            message_thread_id,
            user_id,
            // A webhook payload names no external author.
            author: None,
            sender_external_id: None,
            text: payload.text,
            file_ids: vec![],
            thread_properties,
            adapter_context: serde_json::json!({}),
            external_event_id: Some(external_event_id),
            source_task_id: None,
            // A webhook payload carries no audience distinction: every event
            // goes to every task bound to the source, as it always has.
            audience: Audience::Everyone,
        },
    })
}

pub(crate) async fn receive(
    loops: &dyn LoopReader,
    adapter_delivery: &dyn DeliveryAdmission,
    loop_id: String,
    signature: Option<&str>,
    body: &[u8],
) -> Result<WebhookAccepted, AdapterError> {
    info!(loop_id = %loop_id, "Webhook inbound received");

    let subscription = loops
        .runnable_subscription(&loop_id)
        .await?
        .ok_or_else(|| SharedAdapterError::ConfigNotFound(loop_id.clone()))?;

    if subscription.adapter_kind.as_str() != ADAPTER {
        return Err(SharedAdapterError::BadRequest(format!(
            "Loop {loop_id} is not a webhook Subscription"
        ))
        .into());
    }

    let config = moatless_adapter_model::loops::subscription_context(&subscription);
    let delivery = parse_inbound(&config, body, signature)?;
    let message = delivery.message;
    let thread_id = message
        .thread_id
        .as_deref()
        .unwrap_or(&message.message_thread_id);
    let subject_key = format!("{}:{thread_id}", message.source_id);

    // The Adapter connection is named, not read. Admission runs on the caller's
    // response deadline and must not fail because a connection is missing or
    // concurrently deleted; the handler reads it when it executes and reports a
    // permanent failure if it is gone.
    //
    // Everything below the 200 depends on this write: a failure here must reach
    // the caller as a failure so it re-delivers, and must never be swallowed
    // into an acknowledgment. The routing itself runs in the Adapter
    // delivery worker, which is also where the webhook outcome metric is
    // recorded.
    let request = inbound_delivery_request(
        ADAPTER,
        INBOUND_MESSAGE,
        delivery.delivery_id.clone(),
        InboundDeliveryPayload {
            connection_id: subscription.connection_id.clone(),
            loop_id: subscription.id.clone(),
            subject_kind: SUBJECT_KIND.to_string(),
            subject_key,
            destination_config: serde_json::json!({}),
            event_type: String::new(),
            message,
        },
    )?;
    // This handler has no `receive` to hold the shared `Outcome`/`IngressError`
    // — it answers with `AdapterError` directly and its own body type — so the
    // `AdmitError` is flattened here, by the `From<AdmitError> for AdapterError`
    // impl the kernel owns, rather than through an `IngressError` this handler
    // has no place to hold.
    let admission = admit_delivery(adapter_delivery, request).await?;

    debug!(
        loop_id = %loop_id,
        delivery_id = %delivery.delivery_id,
        admission = admission.as_str(),
        "Webhook delivery admitted for routing"
    );
    Ok(WebhookAccepted {
        delivery_id: delivery.delivery_id,
        status: admission_status(admission),
    })
}

fn admission_status(
    admission: moatless_adapter_model::delivery::Admission,
) -> moatless_api_schemas::webhook::WebhookAdmission {
    match admission {
        moatless_adapter_model::delivery::Admission::Admitted => {
            moatless_api_schemas::webhook::WebhookAdmission::Admitted
        }
        moatless_adapter_model::delivery::Admission::Duplicate => {
            moatless_api_schemas::webhook::WebhookAdmission::Duplicate
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_types::loops::RoutingMode;

    fn test_config(adapter_config: Option<serde_json::Value>) -> LoopContext {
        LoopContext {
            id: "cfg-1".into(),
            adapter_kind: "webhook".to_string(),
            source_id: "my-webhook".into(),
            source_name: None,
            workspace_id: "ws-1".into(),
            routing_mode: RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: false,
            adapter_config,
            ongoing_task_id: None,
            scope: moatless_types::access_control::Scope::User,
            task_name: None,
            tag_ids: vec![],
            skills: Vec::new(),
            run_as_user_id: Some("default-user".into()),
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
            model: None,
            inactive_ttl_days: None,
            created_by: "default-user".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            node_selector: None,
            stop_when_done: false,
        }
    }

    #[test]
    fn parse_basic_payload() {
        let config = test_config(None);
        let body = br#"{"text": "hello world"}"#;
        let msg = parse_inbound(&config, body, None).unwrap().message;

        assert_eq!(msg.text, "hello world");
        assert_eq!(msg.adapter_kind, AdapterKind::Webhook);
        assert_eq!(msg.source_id, "my-webhook");
        assert_eq!(msg.user_id, "default-user");
        assert!(msg.thread_id.is_none());
    }

    #[test]
    fn parse_with_thread_id() {
        let config = test_config(None);
        let body = br#"{"text": "reply", "threadId": "thread-123"}"#;
        let msg = parse_inbound(&config, body, None).unwrap().message;

        assert_eq!(msg.text, "reply");
        assert_eq!(msg.thread_id, Some("thread-123".to_string()));
        assert_eq!(msg.message_thread_id, "thread-123");
    }

    #[test]
    fn parse_with_user_id_override() {
        let config = test_config(Some(serde_json::json!({"user_id": "config-user"})));
        let body = br#"{"text": "hello", "userId": "override-user"}"#;
        let msg = parse_inbound(&config, body, None).unwrap().message;

        assert_eq!(msg.user_id, "override-user");
    }

    #[test]
    fn parse_with_config_default_user() {
        let config = test_config(Some(serde_json::json!({"user_id": "config-user"})));
        let body = br#"{"text": "hello"}"#;
        let msg = parse_inbound(&config, body, None).unwrap().message;

        assert_eq!(msg.user_id, "config-user");
    }

    /// The caller's identifier is what admission keys on, so it has to survive
    /// parsing verbatim — anything derived from it would deduplicate something
    /// other than what the caller named.
    #[test]
    fn a_supplied_delivery_id_is_the_delivery_identity() {
        let config = test_config(None);
        let body = br#"{"text": "hello", "deliveryId": "order-4711"}"#;
        let delivery = parse_inbound(&config, body, None).unwrap();

        assert_eq!(delivery.delivery_id, "order-4711");
    }

    /// Absent, a delivery identity is still required — admission has no other
    /// key. Minting one per request is what makes two identical bodies two
    /// deliveries rather than one, which is the documented contract.
    #[test]
    fn an_absent_delivery_id_is_minted_fresh_per_request() {
        let config = test_config(None);
        let body = br#"{"text": "hello"}"#;

        let first = parse_inbound(&config, body, None).unwrap();
        let second = parse_inbound(&config, body, None).unwrap();

        assert!(!first.delivery_id.is_empty());
        assert_ne!(
            first.delivery_id, second.delivery_id,
            "two deliveries carrying no identifier must not collapse into one"
        );
    }

    /// The worker executes an admitted record at least once, so routing needs a
    /// stable identity to recognise a re-execution by, derived from the
    /// delivery identity rather than the message — two deliveries with the same
    /// text stay distinct.
    #[test]
    fn the_message_carries_a_stable_external_event_id() {
        let config = test_config(None);
        let body = br#"{"text": "hello", "deliveryId": "order-4711"}"#;
        let delivery = parse_inbound(&config, body, None).unwrap();

        assert_eq!(
            delivery.message.external_event_id.as_deref(),
            Some("webhook:delivery:order-4711")
        );

        // Two parses of one identifier agree, so a re-execution of the admitted
        // record looks up the same id.
        let again = parse_inbound(&config, body, None).unwrap();
        assert_eq!(
            again.message.external_event_id,
            delivery.message.external_event_id
        );
    }

    /// A minted identity gets the same treatment — the mint happens once, at
    /// parse time, and is what lands in the admitted payload.
    #[test]
    fn a_minted_delivery_also_carries_an_external_event_id() {
        let config = test_config(None);
        let body = br#"{"text": "hello"}"#;
        let delivery = parse_inbound(&config, body, None).unwrap();

        assert_eq!(
            delivery.message.external_event_id,
            Some(format!("webhook:delivery:{}", delivery.delivery_id))
        );
    }

    /// An empty string is a caller that supplied nothing, spelled differently.
    /// Admitting it verbatim would collapse every such delivery into one
    /// record.
    #[test]
    fn a_blank_delivery_id_is_treated_as_absent() {
        let config = test_config(None);
        let body = br#"{"text": "hello", "deliveryId": "   "}"#;
        let delivery = parse_inbound(&config, body, None).unwrap();

        assert_ne!(delivery.delivery_id.trim(), "");
    }

    /// The identifier is read from inside the signed body, so a tampered
    /// identifier fails verification rather than reaching admission.
    #[test]
    fn the_delivery_id_is_covered_by_the_signature() {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;

        let secret = "my-secret";
        let config = test_config(Some(serde_json::json!({"webhook_secret": secret})));
        let signed = br#"{"text": "hello", "deliveryId": "delivery-1"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signed);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let delivery = parse_inbound(&config, signed, Some(&sig)).unwrap();
        assert_eq!(delivery.delivery_id, "delivery-1");

        let tampered = br#"{"text": "hello", "deliveryId": "delivery-2"}"#;
        let result = parse_inbound(&config, tampered, Some(&sig));
        assert!(
            result.is_err(),
            "swapping the delivery id must invalidate the signature"
        );
    }

    #[test]
    fn parse_empty_text_fails() {
        let config = test_config(None);
        let body = br#"{"text": "  "}"#;
        let result = parse_inbound(&config, body, None);
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_text_fails() {
        let config = test_config(None);
        let body = br#"{"threadId": "t1"}"#;
        let result = parse_inbound(&config, body, None);
        assert!(result.is_err());
    }

    #[test]
    fn signature_required_when_secret_configured() {
        let config = test_config(Some(serde_json::json!({"webhook_secret": "my-secret"})));
        let body = br#"{"text": "hello"}"#;
        let result = parse_inbound(&config, body, None);
        assert!(result.is_err());
    }

    #[test]
    fn signature_verified_when_secret_configured() {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;

        let secret = "my-secret";
        let config = test_config(Some(serde_json::json!({"webhook_secret": secret})));
        let body = br#"{"text": "hello"}"#;

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let msg = parse_inbound(&config, body, Some(&sig)).unwrap().message;
        assert_eq!(msg.text, "hello");
    }
}
