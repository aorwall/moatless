//! Typed GitHub PR provider config — deserialized from `channel_configs.provider_config`.

use serde::{Deserialize, Serialize};

use crate::channel::models::ChannelConfig;

/// Typed representation of `provider_config` JSONB for GitHub PR channels.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GithubPrProviderConfig {
    /// Per-config webhook secret override (takes precedence over global env var).
    pub webhook_secret: Option<String>,
    /// Bot token for API calls. Falls back to env `GITHUB_CHANNEL_TOKEN`,
    /// then to the config owner's provider_token for "github".
    pub bot_token: Option<String>,
    /// Bot's GitHub login (used to filter self-replies).
    pub bot_username: Option<String>,
    /// Filter which events to process. Empty means all supported events.
    /// Possible values: "pull_request", "issue_comment", "pull_request_review_comment",
    ///   "pull_request_review", "check_run"
    #[serde(default)]
    pub events_filter: Vec<String>,
    /// Whether to auto-create channel configs for unconfigured repos from this wildcard template.
    #[serde(default)]
    pub auto_create: bool,
}

impl GithubPrProviderConfig {
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
    fn deserialize_defaults_for_empty_object() {
        let json = serde_json::json!({});
        let cfg: GithubPrProviderConfig = serde_json::from_value(json).unwrap();

        assert!(cfg.webhook_secret.is_none());
        assert!(cfg.bot_token.is_none());
        assert!(cfg.bot_username.is_none());
        assert!(cfg.events_filter.is_empty());
    }

    #[test]
    fn deserialize_full_config() {
        let json = serde_json::json!({
            "webhook_secret": "whsec_123",
            "bot_token": "ghp_abc",
            "bot_username": "my-bot[bot]",
            "events_filter": ["pull_request", "issue_comment"],
        });
        let cfg: GithubPrProviderConfig = serde_json::from_value(json).unwrap();

        assert_eq!(cfg.webhook_secret.as_deref(), Some("whsec_123"));
        assert_eq!(cfg.bot_token.as_deref(), Some("ghp_abc"));
        assert_eq!(cfg.bot_username.as_deref(), Some("my-bot[bot]"));
        assert_eq!(cfg.events_filter, vec!["pull_request", "issue_comment"]);
    }

    #[test]
    fn from_config_with_none_provider_config() {
        let config = ChannelConfig {
            id: "test".into(),
            provider: "github_pr".into(),
            source_id: "owner/repo".into(),
            source_name: None,
            repository_id: "repo1".into(),
            routing_mode: crate::channel::models::RoutingMode::ThreadBased,
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
            created_by: "owner".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let cfg = GithubPrProviderConfig::from_config(&config);
        assert!(cfg.bot_username.is_none());
    }
}
