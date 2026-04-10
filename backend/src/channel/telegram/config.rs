//! Typed Telegram provider config — deserialized from `channel_configs.provider_config`.

use serde::{Deserialize, Serialize};

use crate::channel::models::{ChannelAccessPolicy, ChannelConfig};

/// Typed representation of `provider_config` JSONB for Telegram channels.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TelegramProviderConfig {
    pub bot_username: Option<String>,
    pub secret_token: Option<String>,
    /// DM access policy. Default: Open (backward compat).
    ///
    /// Accepts both the legacy field name `dm_policy` and the shared enum values.
    #[serde(default, alias = "dm_policy")]
    pub dm_access_policy: ChannelAccessPolicy,
    /// Telegram user IDs (numeric strings) allowed to DM.
    #[serde(default)]
    pub dm_allowlist: Vec<String>,
    /// Group access policy. Default: Open.
    ///
    /// Accepts both the legacy field name `group_policy` and the shared enum values.
    /// Note: `Pairing` is not valid for groups; it is treated as `Allowlist`.
    #[serde(default, alias = "group_policy")]
    pub group_access_policy: ChannelAccessPolicy,
    /// Chat IDs allowed for group messages.
    #[serde(default)]
    pub group_allowlist: Vec<String>,
    /// Require @bot mention in group messages.
    #[serde(default)]
    pub require_mention: bool,
}

impl TelegramProviderConfig {
    /// Parse from a `ChannelConfig`'s `provider_config` field.
    /// Returns default config if field is absent or unparseable.
    pub fn from_config(config: &ChannelConfig) -> Self {
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
    fn deserialize_defaults_for_missing_fields() {
        let json = serde_json::json!({ "bot_username": "@mybot" });
        let cfg: TelegramProviderConfig = serde_json::from_value(json).unwrap();

        assert_eq!(cfg.bot_username.as_deref(), Some("@mybot"));
        assert_eq!(cfg.dm_access_policy, ChannelAccessPolicy::Open);
        assert!(cfg.dm_allowlist.is_empty());
        assert_eq!(cfg.group_access_policy, ChannelAccessPolicy::Open);
        assert!(cfg.group_allowlist.is_empty());
        assert!(!cfg.require_mention);
    }

    #[test]
    fn deserialize_full_config_with_new_field_names() {
        let json = serde_json::json!({
            "bot_username": "@testbot",
            "secret_token": "s3cret",
            "dm_access_policy": "pairing",
            "dm_allowlist": ["123", "456"],
            "group_access_policy": "allowlist",
            "group_allowlist": ["-100111"],
            "require_mention": true,
        });
        let cfg: TelegramProviderConfig = serde_json::from_value(json).unwrap();

        assert_eq!(cfg.dm_access_policy, ChannelAccessPolicy::Pairing);
        assert_eq!(cfg.dm_allowlist, vec!["123", "456"]);
        assert_eq!(cfg.group_access_policy, ChannelAccessPolicy::Allowlist);
        assert_eq!(cfg.group_allowlist, vec!["-100111"]);
        assert!(cfg.require_mention);
    }

    #[test]
    fn deserialize_legacy_field_names_via_alias() {
        let json = serde_json::json!({
            "dm_policy": "pairing",
            "group_policy": "allowlist",
        });
        let cfg: TelegramProviderConfig = serde_json::from_value(json).unwrap();

        assert_eq!(cfg.dm_access_policy, ChannelAccessPolicy::Pairing);
        assert_eq!(cfg.group_access_policy, ChannelAccessPolicy::Allowlist);
    }

    #[test]
    fn deserialize_empty_object_gives_defaults() {
        let json = serde_json::json!({});
        let cfg: TelegramProviderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.dm_access_policy, ChannelAccessPolicy::Open);
        assert_eq!(cfg.group_access_policy, ChannelAccessPolicy::Open);
    }

    #[test]
    fn from_config_with_none_provider_config() {
        let config = ChannelConfig {
            id: "test".into(),
            provider: "telegram".into(),
            source_id: "*".into(),
            source_name: None,
            repository_id: "repo1".into(),
            routing_mode: crate::channel::models::RoutingMode::PerMessage,
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
            agent_type: "claude-code".into(),
            created_by: "owner".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let tg = TelegramProviderConfig::from_config(&config);
        assert_eq!(tg.dm_access_policy, ChannelAccessPolicy::Open);
        assert!(tg.bot_username.is_none());
    }
}
