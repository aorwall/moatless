//! Webhook adapter — Adapter trait impl + inbound parsing.
//!
//! Provider config (stored in `trigger_config` JSONB):
//! ```json
//! {
//!   "webhook_secret": "optional-hmac-secret",
//!   "callback_url": "https://example.com/callback",
//!   "callback_headers": { "Authorization": "Bearer xxx" },
//!   "user_id": "default-user-id"
//! }
//! ```

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use moatless_loops::adapters::delivery::Admission;
use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::{
    AdapterKind, Audience, BindingContext, InboundMessage, OutboundReply,
};
use moatless_types::loops::LoopContext;
use moatless_loops::adapters::{
    Adapter, AdapterBindingReply, AdapterMessageReply, AdapterOutboundContent, Adapters,
};

/// Inbound webhook payload.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookPayload {
    /// Message text (required).
    pub text: String,
    /// Thread ID for routing (maps to thread_id in InboundMessage).
    pub thread_id: Option<String>,
    /// User identity override (defaults to config's default user_id).
    pub user_id: Option<String>,
    /// Arbitrary metadata stored in thread properties.
    #[schema(value_type = Option<Object>)]
    pub metadata: Option<serde_json::Value>,
    /// Caller-assigned identity for this delivery. Optional, and what it buys
    /// differs sharply by whether it is supplied:
    ///
    /// - **Supplied.** Two requests carrying the same `deliveryId` are one
    ///   delivery. The second is recognised as a duplicate and does no further
    ///   work — on any replica, and after any restart.
    /// - **Absent.** A fresh identity is minted per request, so sending the same
    ///   body twice is two deliveries and the work is done twice.
    ///
    /// Either way the response means the backend owns the delivery. Only the
    /// deduplication is at the caller's discretion.
    ///
    /// It is read from the request body rather than a header because the
    /// signature covers the body only: an identifier carried outside the signed
    /// content could be altered in transit without invalidating the signature,
    /// which would either replay work as new or collapse two distinct
    /// deliveries into one.
    pub delivery_id: Option<String>,
}

/// What admission did with a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebhookAdmission {
    /// The backend has just taken ownership of this delivery.
    Admitted,
    /// It already owned it. Nothing extra was written and nothing extra runs.
    Duplicate,
}

impl From<Admission> for WebhookAdmission {
    fn from(admission: Admission) -> Self {
        match admission {
            Admission::Admitted => Self::Admitted,
            Admission::Duplicate => Self::Duplicate,
        }
    }
}

/// The answer to an accepted delivery.
///
/// It names the delivery, not a Task: routing has not run when this is written,
/// so no Task exists to name yet.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookAccepted {
    /// The identity the delivery was admitted under — the caller's own
    /// `deliveryId`, or the one minted for it.
    pub delivery_id: String,
    pub status: WebhookAdmission,
}

/// One inbound delivery, verified and parsed, ready to be admitted.
pub(crate) struct WebhookDelivery {
    /// The identity this delivery is admitted under.
    pub(crate) delivery_id: String,
    /// The message the admitted work will route.
    pub(crate) message: InboundMessage,
}

/// Webhook adapter.
pub struct WebhookAdapter;

impl WebhookAdapter {
    /// Register this adapter's capabilities into `adapters`.
    ///
    /// The Webhook adapter has no concrete-only methods, so no handler needs a
    /// concrete handle to it: it has no `ProviderHandles` entry and lives only
    /// as a trait object.
    pub fn register_into(self: Arc<Self>, adapters: &mut Adapters) {
        adapters.register_trait_object(self.clone());
        adapters.register_binding_reply(self.clone());
        adapters.register_message_reply(self);
    }

    pub fn new() -> Self {
        Self
    }

    /// Parse an inbound webhook request into a [`WebhookDelivery`].
    ///
    /// Verifies HMAC signature if `webhook_secret` is configured. The delivery
    /// identity is read from inside the verified body, so a caller that signs
    /// its request signs its identifier too.
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
            return Err(
                SharedAdapterError::BadRequest("Missing required field: text".into()).into(),
            );
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
}

impl Default for WebhookAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Adapter for WebhookAdapter {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::Webhook
    }
}

impl AdapterOutboundContent for WebhookAdapter {}

#[async_trait::async_trait]
impl AdapterBindingReply for WebhookAdapter {
    async fn send_reply(
        &self,
        thread: &BindingContext,
        config: &LoopContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let callback_url = config
            .adapter_config
            .as_ref()
            .and_then(|c| c.get("callback_url"))
            .and_then(|v| v.as_str());

        let Some(callback_url) = callback_url else {
            debug!(loop_id = %config.id, "No callback_url configured, skipping outbound webhook");
            return Ok(None);
        };

        let callback_headers: std::collections::HashMap<String, String> = config
            .adapter_config
            .as_ref()
            .and_then(|c| c.get("callback_headers"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let body = serde_json::json!({
            "task_id": thread.task_id,
            "thread_id": thread.subject_key,
            "text": message.text,
            "provenance": message.provenance,
            "is_error": message.is_error,
        });

        let client = reqwest::Client::new();
        let mut req = client.post(callback_url).json(&body);
        for (key, value) in &callback_headers {
            req = req.header(key.as_str(), value.as_str());
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                debug!(loop_id = %config.id, url = %callback_url, "Webhook callback sent successfully");
                Ok(None)
            }
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                warn!(loop_id = %config.id, url = %callback_url, status = %status, body = %text, "Webhook callback failed");
                Err(SharedAdapterError::AdapterApi(format!(
                    "Webhook callback returned {status}: {text}"
                ))
                .into())
            }
            Err(e) => {
                warn!(loop_id = %config.id, url = %callback_url, "Webhook callback request failed");
                Err(SharedAdapterError::provider_unreachable("Webhook callback", e).into())
            }
        }
    }
}

#[async_trait::async_trait]
impl AdapterMessageReply for WebhookAdapter {
    async fn reply_to_message(
        &self,
        binding: &moatless_adapter_model::models::TaskBinding,
        connection: &moatless_adapter_model::models::AdapterConnection,
        rule: Option<&moatless_adapter_model::models::SubscriptionMatch>,
        _target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        self.send_reply_for_binding(binding, connection, rule, message)
            .await
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
            repository_id: "repo-1".into(),
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
            agent_type: "claude-code".into(),
            inactive_ttl_days: None,
            created_by: "default-user".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn parse_basic_payload() {
        let config = test_config(None);
        let body = br#"{"text": "hello world"}"#;
        let msg = WebhookAdapter::parse_inbound(&config, body, None)
            .unwrap()
            .message;

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
        let msg = WebhookAdapter::parse_inbound(&config, body, None)
            .unwrap()
            .message;

        assert_eq!(msg.text, "reply");
        assert_eq!(msg.thread_id, Some("thread-123".to_string()));
        assert_eq!(msg.message_thread_id, "thread-123");
    }

    #[test]
    fn parse_with_user_id_override() {
        let config = test_config(Some(serde_json::json!({"user_id": "config-user"})));
        let body = br#"{"text": "hello", "userId": "override-user"}"#;
        let msg = WebhookAdapter::parse_inbound(&config, body, None)
            .unwrap()
            .message;

        assert_eq!(msg.user_id, "override-user");
    }

    #[test]
    fn parse_with_config_default_user() {
        let config = test_config(Some(serde_json::json!({"user_id": "config-user"})));
        let body = br#"{"text": "hello"}"#;
        let msg = WebhookAdapter::parse_inbound(&config, body, None)
            .unwrap()
            .message;

        assert_eq!(msg.user_id, "config-user");
    }

    /// The caller's identifier is what admission keys on, so it has to survive
    /// parsing verbatim — anything derived from it would deduplicate something
    /// other than what the caller named.
    #[test]
    fn a_supplied_delivery_id_is_the_delivery_identity() {
        let config = test_config(None);
        let body = br#"{"text": "hello", "deliveryId": "order-4711"}"#;
        let delivery = WebhookAdapter::parse_inbound(&config, body, None).unwrap();

        assert_eq!(delivery.delivery_id, "order-4711");
    }

    /// Absent, a delivery identity is still required — admission has no other
    /// key. Minting one per request is what makes two identical bodies two
    /// deliveries rather than one, which is the documented contract.
    #[test]
    fn an_absent_delivery_id_is_minted_fresh_per_request() {
        let config = test_config(None);
        let body = br#"{"text": "hello"}"#;

        let first = WebhookAdapter::parse_inbound(&config, body, None).unwrap();
        let second = WebhookAdapter::parse_inbound(&config, body, None).unwrap();

        assert!(!first.delivery_id.is_empty());
        assert_ne!(
            first.delivery_id, second.delivery_id,
            "two deliveries carrying no identifier must not collapse into one"
        );
    }

    /// The worker executes an admitted record at least once, so routing needs a
    /// stable identity to recognise a re-execution by. It is derived from the
    /// delivery identity rather than the message, so two deliveries carrying the
    /// same text stay distinct.
    #[test]
    fn the_message_carries_a_stable_external_event_id() {
        let config = test_config(None);
        let body = br#"{"text": "hello", "deliveryId": "order-4711"}"#;
        let delivery = WebhookAdapter::parse_inbound(&config, body, None).unwrap();

        assert_eq!(
            delivery.message.external_event_id.as_deref(),
            Some("webhook:delivery:order-4711")
        );

        // Two parses of one identifier agree, so a re-execution of the admitted
        // record looks up the same id.
        let again = WebhookAdapter::parse_inbound(&config, body, None).unwrap();
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
        let delivery = WebhookAdapter::parse_inbound(&config, body, None).unwrap();

        assert_eq!(
            delivery.message.external_event_id,
            Some(format!("webhook:delivery:{}", delivery.delivery_id))
        );
    }

    /// An empty string is a caller that supplied nothing, spelled differently.
    /// Admitting it verbatim would collapse every such delivery into one record.
    #[test]
    fn a_blank_delivery_id_is_treated_as_absent() {
        let config = test_config(None);
        let body = br#"{"text": "hello", "deliveryId": "   "}"#;
        let delivery = WebhookAdapter::parse_inbound(&config, body, None).unwrap();

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

        let delivery = WebhookAdapter::parse_inbound(&config, signed, Some(&sig)).unwrap();
        assert_eq!(delivery.delivery_id, "delivery-1");

        let tampered = br#"{"text": "hello", "deliveryId": "delivery-2"}"#;
        let result = WebhookAdapter::parse_inbound(&config, tampered, Some(&sig));
        assert!(
            result.is_err(),
            "swapping the delivery id must invalidate the signature"
        );
    }

    #[test]
    fn parse_empty_text_fails() {
        let config = test_config(None);
        let body = br#"{"text": "  "}"#;
        let result = WebhookAdapter::parse_inbound(&config, body, None);
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_text_fails() {
        let config = test_config(None);
        let body = br#"{"threadId": "t1"}"#;
        let result = WebhookAdapter::parse_inbound(&config, body, None);
        assert!(result.is_err());
    }

    #[test]
    fn signature_required_when_secret_configured() {
        let config = test_config(Some(serde_json::json!({"webhook_secret": "my-secret"})));
        let body = br#"{"text": "hello"}"#;
        let result = WebhookAdapter::parse_inbound(&config, body, None);
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

        let msg = WebhookAdapter::parse_inbound(&config, body, Some(&sig))
            .unwrap()
            .message;
        assert_eq!(msg.text, "hello");
    }
}
