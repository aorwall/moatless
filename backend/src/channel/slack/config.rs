//! Typed Slack provider config — deserialized from `trigger_config`.

use serde::{Deserialize, Serialize};

use crate::channel::models::{ChannelAccessPolicy, ChannelTrigger, TriggerContext};

/// Typed representation of `provider_config` JSONB for Slack channels.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlackProviderConfig {
    /// When to process messages: mention-only (default) or all messages.
    #[serde(default)]
    pub trigger: ChannelTrigger,
    /// Access policy for this Slack channel.
    #[serde(default)]
    pub access_policy: ChannelAccessPolicy,
    /// Slack channel IDs allowed (when access_policy = Allowlist).
    #[serde(default)]
    pub channel_allowlist: Vec<String>,
    /// Slack user IDs allowed (when access_policy = Allowlist).
    #[serde(default)]
    pub user_allowlist: Vec<String>,
    /// Max thread messages to fetch for agent context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_context_limit: Option<u64>,
}

impl Default for SlackProviderConfig {
    fn default() -> Self {
        Self {
            trigger: ChannelTrigger::MentionOnly,
            access_policy: ChannelAccessPolicy::Open,
            channel_allowlist: Vec::new(),
            user_allowlist: Vec::new(),
            thread_context_limit: None,
        }
    }
}

impl SlackProviderConfig {
    /// Parse from a `TriggerContext`'s `provider_config` field.
    /// Returns default config if field is absent or unparseable.
    ///
    /// Also supports legacy untyped configs where `trigger` was a plain string.
    pub fn from_config(config: &TriggerContext) -> Self {
        config
            .provider_config
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = SlackProviderConfig::default();
        assert_eq!(cfg.trigger, ChannelTrigger::MentionOnly);
        assert_eq!(cfg.access_policy, ChannelAccessPolicy::Open);
        assert!(cfg.channel_allowlist.is_empty());
        assert!(cfg.user_allowlist.is_empty());
        assert!(cfg.thread_context_limit.is_none());
    }

    #[test]
    fn deserialize_legacy_trigger_string() {
        let json = serde_json::json!({
            "trigger": "all_messages"
        });
        let cfg: SlackProviderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.trigger, ChannelTrigger::AllMessages);
        assert_eq!(cfg.access_policy, ChannelAccessPolicy::Open);
    }

    #[test]
    fn deserialize_full_config() {
        let json = serde_json::json!({
            "trigger": "mention_only",
            "access_policy": "allowlist",
            "channel_allowlist": ["C123", "C456"],
            "user_allowlist": ["U789"],
            "thread_context_limit": 50
        });
        let cfg: SlackProviderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.trigger, ChannelTrigger::MentionOnly);
        assert_eq!(cfg.access_policy, ChannelAccessPolicy::Allowlist);
        assert_eq!(cfg.channel_allowlist, vec!["C123", "C456"]);
        assert_eq!(cfg.user_allowlist, vec!["U789"]);
        assert_eq!(cfg.thread_context_limit, Some(50));
    }

    #[test]
    fn deserialize_empty_gives_defaults() {
        let json = serde_json::json!({});
        let cfg: SlackProviderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.trigger, ChannelTrigger::MentionOnly);
        assert_eq!(cfg.access_policy, ChannelAccessPolicy::Open);
    }

    #[test]
    fn from_config_with_none_provider_config() {
        let config = TriggerContext {
            id: "test".into(),
            provider: "slack".into(),
            source_id: "C123".into(),
            source_name: None,
            repository_id: "repo1".into(),
            routing_mode: crate::channel::models::RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: false,
            provider_config: None,
            ongoing_task_id: None,
            scope: crate::scope::Scope::User,
            team_id: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            created_by: "owner".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let cfg = SlackProviderConfig::from_config(&config);
        assert_eq!(cfg.trigger, ChannelTrigger::MentionOnly);
        assert_eq!(cfg.access_policy, ChannelAccessPolicy::Open);
    }
}
