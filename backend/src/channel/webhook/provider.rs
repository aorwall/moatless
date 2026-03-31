//! Webhook channel provider — ChannelProvider trait impl + inbound parsing.
//!
//! Provider config (stored in `channel_configs.provider_config` JSONB):
//! ```json
//! {
//!   "webhook_secret": "optional-hmac-secret",
//!   "callback_url": "https://example.com/callback",
//!   "callback_headers": { "Authorization": "Bearer xxx" },
//!   "user_id": "default-user-id"
//! }
//! ```

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::channel::error::ChannelError;
use crate::channel::models::{
    ChannelConfig, ChannelThread, InboundMessage, OutboundReply,
};
use crate::channel::provider::{ChannelProvider, ChannelType};
use crate::db::DbPool;

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
}

/// Webhook channel provider.
pub struct WebhookProvider;

impl WebhookProvider {
    pub fn new() -> Self {
        Self
    }

    /// Parse an inbound webhook request into an `InboundMessage`.
    ///
    /// Verifies HMAC signature if `webhook_secret` is configured.
    pub fn parse_inbound(
        config: &ChannelConfig,
        body: &[u8],
        signature: Option<&str>,
    ) -> Result<InboundMessage, ChannelError> {
        // 1. Verify signature if webhook_secret is configured
        let webhook_secret = config
            .provider_config
            .as_ref()
            .and_then(|c| c.get("webhook_secret"))
            .and_then(|v| v.as_str());

        if let Some(secret) = webhook_secret {
            let sig = signature.ok_or(ChannelError::SignatureInvalid)?;
            super::signature::verify_signature(secret, body, sig)?;
        }

        // 2. Parse payload
        let payload: WebhookPayload = serde_json::from_slice(body)
            .map_err(|e| ChannelError::BadRequest(format!("Invalid JSON: {e}")))?;

        if payload.text.trim().is_empty() {
            return Err(ChannelError::BadRequest(
                "Missing required field: text".into(),
            ));
        }

        // 3. Determine user_id
        let user_id = payload
            .user_id
            .or_else(|| {
                config
                    .provider_config
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
            "config_id": config.id,
            "metadata": payload.metadata,
        });

        Ok(InboundMessage {
            provider: ChannelType::Webhook,
            source_id: config.source_id.clone(),
            thread_id: payload.thread_id,
            message_thread_id,
            user_id,
            text: payload.text,
            file_ids: vec![],
            thread_properties,
            provider_context: serde_json::json!({}),
            channel_external_id: None,
        })
    }
}

impl Default for WebhookProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ChannelProvider for WebhookProvider {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Webhook
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn resolve_user(
        &self,
        identity: &str,
        _db: &DbPool,
    ) -> Result<Option<String>, ChannelError> {
        // Webhook users are passed directly as system user IDs
        Ok(Some(identity.to_string()))
    }

    async fn send_reply(
        &self,
        thread: &ChannelThread,
        config: &ChannelConfig,
        message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError> {
        let callback_url = config
            .provider_config
            .as_ref()
            .and_then(|c| c.get("callback_url"))
            .and_then(|v| v.as_str());

        let Some(callback_url) = callback_url else {
            debug!(config_id = %config.id, "No callback_url configured, skipping outbound webhook");
            return Ok(None);
        };

        let callback_headers: std::collections::HashMap<String, String> = config
            .provider_config
            .as_ref()
            .and_then(|c| c.get("callback_headers"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let body = serde_json::json!({
            "task_id": thread.task_id,
            "thread_id": thread.external_thread_id,
            "text": message.text,
            "task_url": message.task_url,
            "is_error": message.is_error,
        });

        let client = reqwest::Client::new();
        let mut req = client.post(callback_url).json(&body);
        for (key, value) in &callback_headers {
            req = req.header(key.as_str(), value.as_str());
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                debug!(config_id = %config.id, url = %callback_url, "Webhook callback sent successfully");
                Ok(None)
            }
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                warn!(config_id = %config.id, url = %callback_url, status = %status, body = %text, "Webhook callback failed");
                Err(ChannelError::ProviderApi(format!(
                    "Webhook callback returned {status}: {text}"
                )))
            }
            Err(e) => {
                warn!(config_id = %config.id, url = %callback_url, error = %e, "Webhook callback request failed");
                Err(ChannelError::ProviderApi(format!(
                    "Webhook callback failed: {e}"
                )))
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::models::RoutingMode;

    fn test_config(provider_config: Option<serde_json::Value>) -> ChannelConfig {
        ChannelConfig {
            id: "cfg-1".into(),
            provider: "webhook".into(),
            source_id: "my-webhook".into(),
            source_name: None,
            repository_id: "repo-1".into(),
            routing_mode: RoutingMode::PerMessage,
            prompt_template: None,
            active: true,
            provider_config,
            ongoing_task_id: None,
            response_config_id: None,
            scope: crate::scope::Scope::User,
            team_id: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            created_by: "default-user".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn parse_basic_payload() {
        let config = test_config(None);
        let body = br#"{"text": "hello world"}"#;
        let msg = WebhookProvider::parse_inbound(&config, body, None).unwrap();

        assert_eq!(msg.text, "hello world");
        assert_eq!(msg.provider, ChannelType::Webhook);
        assert_eq!(msg.source_id, "my-webhook");
        assert_eq!(msg.user_id, "default-user"); // falls back to created_by
        assert!(msg.thread_id.is_none());
    }

    #[test]
    fn parse_with_thread_id() {
        let config = test_config(None);
        let body = br#"{"text": "reply", "threadId": "thread-123"}"#;
        let msg = WebhookProvider::parse_inbound(&config, body, None).unwrap();

        assert_eq!(msg.text, "reply");
        assert_eq!(msg.thread_id, Some("thread-123".to_string()));
        assert_eq!(msg.message_thread_id, "thread-123");
    }

    #[test]
    fn parse_with_user_id_override() {
        let config = test_config(Some(serde_json::json!({"user_id": "config-user"})));
        let body = br#"{"text": "hello", "userId": "override-user"}"#;
        let msg = WebhookProvider::parse_inbound(&config, body, None).unwrap();

        assert_eq!(msg.user_id, "override-user");
    }

    #[test]
    fn parse_with_config_default_user() {
        let config = test_config(Some(serde_json::json!({"user_id": "config-user"})));
        let body = br#"{"text": "hello"}"#;
        let msg = WebhookProvider::parse_inbound(&config, body, None).unwrap();

        assert_eq!(msg.user_id, "config-user");
    }

    #[test]
    fn parse_empty_text_fails() {
        let config = test_config(None);
        let body = br#"{"text": "  "}"#;
        let result = WebhookProvider::parse_inbound(&config, body, None);
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_text_fails() {
        let config = test_config(None);
        let body = br#"{"threadId": "t1"}"#;
        let result = WebhookProvider::parse_inbound(&config, body, None);
        assert!(result.is_err());
    }

    #[test]
    fn signature_required_when_secret_configured() {
        let config = test_config(Some(serde_json::json!({"webhook_secret": "my-secret"})));
        let body = br#"{"text": "hello"}"#;
        let result = WebhookProvider::parse_inbound(&config, body, None);
        assert!(result.is_err());
    }

    #[test]
    fn signature_verified_when_secret_configured() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let secret = "my-secret";
        let config = test_config(Some(serde_json::json!({"webhook_secret": secret})));
        let body = br#"{"text": "hello"}"#;

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let msg = WebhookProvider::parse_inbound(&config, body, Some(&sig)).unwrap();
        assert_eq!(msg.text, "hello");
    }
}
