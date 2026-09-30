//! Webhook adapter capabilities and outbound callbacks.
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

use tracing::{debug, warn};

use moatless_adapter_model::binding::BindingContext;
use moatless_adapter_model::capabilities::{
    Adapter, AdapterBindingReply, AdapterMessageReply, AdapterOutboundContent,
};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::outbound::OutboundReply;
use moatless_adapter_model::registry::Adapters;
use moatless_types::adapter::AdapterKind;
use moatless_types::http::HttpTimeouts;
use moatless_types::loops::LoopContext;

pub use moatless_api_schemas::webhook::{WebhookAccepted, WebhookAdmission, WebhookPayload};

/// Webhook adapter.
pub struct WebhookAdapter {
    /// Posts each callback to a third party's endpoint, so a stalled one fails
    /// the reply rather than holding it open.
    http: reqwest::Client,
}

impl WebhookAdapter {
    /// Register `WebhookAdapter`'s capabilities into `adapters`.
    ///
    /// It has no concrete-only methods, so it has no `ProviderHandles` entry
    /// and lives only as a trait object.
    pub fn register_into(self: Arc<Self>, adapters: &mut Adapters) {
        adapters.register_trait_object(self.clone());
        adapters.register_binding_reply(self.clone());
        adapters.register_message_reply(self);
    }

    #[expect(
        clippy::disallowed_methods,
        clippy::expect_used,
        reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
    )]
    pub fn new(timeouts: HttpTimeouts) -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(timeouts.connect)
                .timeout(timeouts.total)
                .build()
                .expect("the TLS backend initialises"),
        }
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

        let mut req = self.http.post(callback_url).json(&body);
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
        binding: &moatless_types::adapter::TaskBinding,
        connection: &moatless_types::adapter::AdapterConnection,
        rule: Option<&moatless_types::loops::SubscriptionMatch>,
        _target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        self.send_reply_for_binding(binding, connection, rule, message)
            .await
    }
}
