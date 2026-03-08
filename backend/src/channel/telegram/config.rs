//! Typed Telegram provider config — deserialized from `channel_configs.provider_config`.

use serde::{Deserialize, Serialize};

use crate::channel::models::ChannelConfig;

/// Typed representation of `provider_config` JSONB for Telegram channels.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TelegramProviderConfig {
    pub bot_username: Option<String>,
    pub secret_token: Option<String>,
    /// DM access policy. Default: Open (backward compat).
    #[serde(default)]
    pub dm_policy: DmPolicy,
    /// Telegram user IDs (numeric strings) allowed to DM.
    #[serde(default)]
    pub dm_allowlist: Vec<String>,
    /// Group access policy. Default: Open.
    #[serde(default)]
    pub group_policy: GroupPolicy,
    /// Chat IDs allowed for group messages.
    #[serde(default)]
    pub group_allowlist: Vec<String>,
    /// Require @bot mention in group messages.
    #[serde(default)]
    pub require_mention: bool,
}

/// DM access policy.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DmPolicy {
    /// Accept messages from any user.
    #[default]
    Open,
    /// Reject all DMs.
    Disabled,
    /// Accept only from users in dm_allowlist.
    Allowlist,
    /// Allowlist + prompt unknown users with a pairing code.
    Pairing,
}

/// Group access policy.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GroupPolicy {
    /// Accept messages from any group.
    #[default]
    Open,
    /// Reject all group messages.
    Disabled,
    /// Accept only from groups in group_allowlist.
    Allowlist,
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
        assert_eq!(cfg.dm_policy, DmPolicy::Open);
        assert!(cfg.dm_allowlist.is_empty());
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
        assert!(cfg.group_allowlist.is_empty());
        assert!(!cfg.require_mention);
    }

    #[test]
    fn deserialize_full_config() {
        let json = serde_json::json!({
            "bot_username": "@testbot",
            "secret_token": "s3cret",
            "dm_policy": "pairing",
            "dm_allowlist": ["123", "456"],
            "group_policy": "allowlist",
            "group_allowlist": ["-100111"],
            "require_mention": true,
        });
        let cfg: TelegramProviderConfig = serde_json::from_value(json).unwrap();

        assert_eq!(cfg.dm_policy, DmPolicy::Pairing);
        assert_eq!(cfg.dm_allowlist, vec!["123", "456"]);
        assert_eq!(cfg.group_policy, GroupPolicy::Allowlist);
        assert_eq!(cfg.group_allowlist, vec!["-100111"]);
        assert!(cfg.require_mention);
    }

    #[test]
    fn deserialize_empty_object_gives_defaults() {
        let json = serde_json::json!({});
        let cfg: TelegramProviderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.dm_policy, DmPolicy::Open);
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
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
            provider_config: None,
            ongoing_task_id: None,
            response_config_id: None,
            scope: crate::scope::Scope::User,
            team_id: None,
            created_by: "owner".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let tg = TelegramProviderConfig::from_config(&config);
        assert_eq!(tg.dm_policy, DmPolicy::Open);
        assert!(tg.bot_username.is_none());
    }
}
