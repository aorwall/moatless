//! Cron channel provider — ChannelProvider trait implementation.
//!
//! Provider config (stored in `channel_configs.provider_config` JSONB):
//! ```json
//! {
//!   "schedule": "0 0 9 * * MON-FRI *",
//!   "message": "Run daily status check",
//!   "user_id": "system"
//! }
//! ```
//!
//! The cron provider generates `InboundMessage`s on schedule.
//! Outbound `send_reply` is a no-op — use `response_config_id` to route
//! replies to another channel (e.g., Slack or webhook).

use crate::channel::error::ChannelError;
use crate::channel::models::{
    ChannelConfig, ChannelThread, InboundMessage, OutboundReply,
};
use crate::channel::provider::{ChannelProvider, ChannelType};
use crate::db::DbPool;

/// Cron channel provider.
pub struct CronProvider;

impl CronProvider {
    pub fn new() -> Self {
        Self
    }

    /// Generate an inbound message from a cron trigger.
    ///
    /// Called by the cron scheduler when a schedule fires.
    pub fn generate_message(config: &ChannelConfig) -> InboundMessage {
        let message = config
            .provider_config
            .as_ref()
            .and_then(|c| c.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("Scheduled task trigger");

        let user_id = config
            .provider_config
            .as_ref()
            .and_then(|c| c.get("user_id"))
            .and_then(|v| v.as_str())
            .map_or_else(|| config.effective_user_id().to_string(), String::from);

        let thread_id = uuid::Uuid::new_v4().to_string();

        InboundMessage {
            provider: ChannelType::Cron,
            source_id: config.source_id.clone(),
            thread_id: None,
            message_thread_id: thread_id,
            user_id,
            text: message.to_string(),
            file_ids: vec![],
            thread_properties: serde_json::json!({"trigger": "cron"}),
            provider_context: serde_json::json!({}),
            channel_external_id: None,
        }
    }
}

impl Default for CronProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ChannelProvider for CronProvider {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Cron
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn resolve_user(
        &self,
        identity: &str,
        _db: &DbPool,
    ) -> Result<Option<String>, ChannelError> {
        Ok(Some(identity.to_string()))
    }

    async fn send_reply(
        &self,
        _thread: &ChannelThread,
        _config: &ChannelConfig,
        _message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError> {
        // Cron provider has no outbound channel — use response_config_id
        // to route replies to another channel.
        Ok(None)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::models::RoutingMode;

    #[test]
    fn generate_message_with_config() {
        let config = ChannelConfig {
            id: "cron-1".into(),
            provider: "cron".into(),
            source_id: "daily-check".into(),
            source_name: Some("Daily Check".into()),
            repository_id: "repo-1".into(),
            routing_mode: RoutingMode::PerMessage,
            prompt_template: None,
            active: true,
            allow_direct_send: false,
            provider_config: Some(serde_json::json!({
                "schedule": "0 0 9 * * MON-FRI *",
                "message": "Run daily status check",
                "user_id": "system-user"
            })),
            ongoing_task_id: None,
            response_config_id: None,
            scope: crate::scope::Scope::User,
            team_id: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            created_by: "admin".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let msg = CronProvider::generate_message(&config);

        assert_eq!(msg.text, "Run daily status check");
        assert_eq!(msg.user_id, "system-user");
        assert_eq!(msg.provider, ChannelType::Cron);
        assert_eq!(msg.source_id, "daily-check");
        assert!(msg.thread_id.is_none());
    }

    #[test]
    fn generate_message_default_fallbacks() {
        let config = ChannelConfig {
            id: "cron-2".into(),
            provider: "cron".into(),
            source_id: "weekly".into(),
            source_name: None,
            repository_id: "repo-1".into(),
            routing_mode: RoutingMode::PerMessage,
            prompt_template: None,
            active: true,
            allow_direct_send: false,
            provider_config: None,
            ongoing_task_id: None,
            response_config_id: None,
            scope: crate::scope::Scope::User,
            team_id: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            created_by: "fallback-user".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let msg = CronProvider::generate_message(&config);

        assert_eq!(msg.text, "Scheduled task trigger");
        assert_eq!(msg.user_id, "fallback-user");
    }
}
