//! Typed Telegram provider config — deserialized from `trigger_config`.

use serde::{Deserialize, Serialize};

use moatless_adapter_model::access::SourceAccessPolicy;
use moatless_types::loops::LoopContext;

/// Typed representation of `adapter_config` JSONB for Telegram channels.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TelegramAdapterConfig {
    pub bot_username: Option<String>,
    pub secret_token: Option<String>,
    /// DM access policy. Default: Open (backward compat).
    ///
    /// Accepts both the legacy field name `dm_policy` and the shared enum values.
    #[serde(default, alias = "dm_policy")]
    pub dm_access_policy: SourceAccessPolicy,
    /// Telegram user IDs (numeric strings) allowed to DM.
    #[serde(default)]
    pub dm_allowlist: Vec<String>,
    /// Group access policy. Default: Open.
    ///
    /// Accepts both the legacy field name `group_policy` and the shared enum values.
    /// Note: `Pairing` is not valid for groups; it is treated as `Allowlist`.
    #[serde(default, alias = "group_policy")]
    pub group_access_policy: SourceAccessPolicy,
    /// Chat IDs allowed for group messages.
    #[serde(default)]
    pub group_allowlist: Vec<String>,
    /// Require @bot mention in group messages.
    #[serde(default)]
    pub require_mention: bool,
}

impl TelegramAdapterConfig {
    /// Parse from a `LoopContext`'s `adapter_config` field.
    /// Returns default config if field is absent or unparseable.
    pub fn from_config(config: &LoopContext) -> Self {
        config
            .adapter_config
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
        let cfg: TelegramAdapterConfig = serde_json::from_value(json).unwrap();

        assert_eq!(cfg.bot_username.as_deref(), Some("@mybot"));
        assert_eq!(cfg.dm_access_policy, SourceAccessPolicy::Open);
        assert!(cfg.dm_allowlist.is_empty());
        assert_eq!(cfg.group_access_policy, SourceAccessPolicy::Open);
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
        let cfg: TelegramAdapterConfig = serde_json::from_value(json).unwrap();

        assert_eq!(cfg.dm_access_policy, SourceAccessPolicy::Pairing);
        assert_eq!(cfg.dm_allowlist, vec!["123", "456"]);
        assert_eq!(cfg.group_access_policy, SourceAccessPolicy::Allowlist);
        assert_eq!(cfg.group_allowlist, vec!["-100111"]);
        assert!(cfg.require_mention);
    }

    #[test]
    fn deserialize_legacy_field_names_via_alias() {
        let json = serde_json::json!({
            "dm_policy": "pairing",
            "group_policy": "allowlist",
        });
        let cfg: TelegramAdapterConfig = serde_json::from_value(json).unwrap();

        assert_eq!(cfg.dm_access_policy, SourceAccessPolicy::Pairing);
        assert_eq!(cfg.group_access_policy, SourceAccessPolicy::Allowlist);
    }

    #[test]
    fn deserialize_empty_object_gives_defaults() {
        let json = serde_json::json!({});
        let cfg: TelegramAdapterConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.dm_access_policy, SourceAccessPolicy::Open);
        assert_eq!(cfg.group_access_policy, SourceAccessPolicy::Open);
    }

    #[test]
    fn from_config_with_none_adapter_config() {
        let config = LoopContext {
            id: "test".into(),
            adapter_kind: "telegram".to_string(),
            source_id: "*".into(),
            source_name: None,
            repository_id: "repo1".into(),
            workspace_id: None,
            routing_mode: moatless_types::loops::RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: false,
            adapter_config: None,
            ongoing_task_id: None,
            scope: moatless_types::access_control::Scope::User,
            task_name: None,
            tag_ids: vec![],
            skills: Vec::new(),
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            model: None,
            inactive_ttl_days: None,
            created_by: "owner".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let tg = TelegramAdapterConfig::from_config(&config);
        assert_eq!(tg.dm_access_policy, SourceAccessPolicy::Open);
        assert!(tg.bot_username.is_none());
    }
}
