//! Typed Slack provider config — deserialized from `trigger_config`.

use serde::{Deserialize, Serialize};

use moatless_adapter_model::access::MessageTrigger;
use moatless_adapter_model::access::SourceAccessPolicy;
use moatless_types::loops::LoopContext;

/// Typed representation of `adapter_config` JSONB for Slack channels.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlackAdapterConfig {
    /// When to process messages: mention-only (default) or all messages.
    #[serde(default)]
    pub trigger: MessageTrigger,
    /// Access policy for this Slack channel.
    #[serde(default)]
    pub access_policy: SourceAccessPolicy,
    /// Slack channel IDs allowed (when `access_policy` = Allowlist).
    #[serde(default)]
    pub channel_allowlist: Vec<String>,
    /// Slack user IDs allowed (when `access_policy` = Allowlist).
    #[serde(default)]
    pub user_allowlist: Vec<String>,
    /// Max thread messages to fetch for agent context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_context_limit: Option<u64>,
    /// Allow messages from OTHER bots (e.g. a CI notifier) to be processed. The
    /// agent's own posts are still suppressed at the binding layer (`self_echo`
    /// by message ts), so enabling this does not create echo loops.
    #[serde(default)]
    pub allow_bot_messages: bool,
    /// Render the task-provenance footer on the first reply in a thread.
    /// Defaults to on: a message nobody can trace back is the worse default.
    #[serde(default = "default_true")]
    pub task_link: bool,
}

fn default_true() -> bool {
    true
}

impl Default for SlackAdapterConfig {
    fn default() -> Self {
        Self {
            trigger: MessageTrigger::MentionOnly,
            access_policy: SourceAccessPolicy::Open,
            channel_allowlist: Vec::new(),
            user_allowlist: Vec::new(),
            thread_context_limit: None,
            allow_bot_messages: false,
            task_link: true,
        }
    }
}

impl SlackAdapterConfig {
    /// Parse from a `LoopContext`'s `adapter_config` field, defaulting when
    /// absent or unparseable.
    ///
    /// Also supports legacy untyped configs where `trigger` was a plain string.
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
    fn default_config() {
        let cfg = SlackAdapterConfig::default();
        assert_eq!(cfg.trigger, MessageTrigger::MentionOnly);
        assert_eq!(cfg.access_policy, SourceAccessPolicy::Open);
        assert!(cfg.channel_allowlist.is_empty());
        assert!(cfg.user_allowlist.is_empty());
        assert!(cfg.thread_context_limit.is_none());
        assert!(!cfg.allow_bot_messages);
    }

    #[test]
    fn deserialize_legacy_trigger_string() {
        let json = serde_json::json!({
            "trigger": "all_messages"
        });
        let cfg: SlackAdapterConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.trigger, MessageTrigger::AllMessages);
        assert_eq!(cfg.access_policy, SourceAccessPolicy::Open);
    }

    #[test]
    fn deserialize_full_config() {
        let json = serde_json::json!({
            "trigger": "mention_only",
            "access_policy": "allowlist",
            "channel_allowlist": ["C123", "C456"],
            "user_allowlist": ["U789"],
            "thread_context_limit": 50,
            "allow_bot_messages": true
        });
        let cfg: SlackAdapterConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.trigger, MessageTrigger::MentionOnly);
        assert_eq!(cfg.access_policy, SourceAccessPolicy::Allowlist);
        assert_eq!(cfg.channel_allowlist, vec!["C123", "C456"]);
        assert_eq!(cfg.user_allowlist, vec!["U789"]);
        assert_eq!(cfg.thread_context_limit, Some(50));
        assert!(cfg.allow_bot_messages);
    }

    /// A Loop saved with the retired `pairing` policy keeps its allowlists.
    #[test]
    fn a_stored_pairing_policy_reads_as_allowlist() {
        let json = serde_json::json!({
            "access_policy": "pairing",
            "user_allowlist": ["U789"]
        });
        let cfg: SlackAdapterConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.access_policy, SourceAccessPolicy::Allowlist);
        assert_eq!(cfg.user_allowlist, vec!["U789"]);
    }

    #[test]
    fn deserialize_empty_gives_defaults() {
        let json = serde_json::json!({});
        let cfg: SlackAdapterConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.trigger, MessageTrigger::MentionOnly);
        assert_eq!(cfg.access_policy, SourceAccessPolicy::Open);
    }

    #[test]
    fn from_config_with_none_adapter_config() {
        let config = LoopContext {
            id: "test".into(),
            adapter_kind: "slack".to_string(),
            source_id: "C123".into(),
            source_name: None,
            workspace_id: "ws1".into(),
            routing_mode: moatless_types::loops::RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: false,
            adapter_config: None,
            ongoing_task_id: None,
            scope: moatless_types::access_control::Scope::User,
            task_name: None,
            tag_ids: vec![],
            memory_preload: Vec::new(),
            agent_properties: Default::default(),
            skills: Vec::new(),
            run_as_user_id: None,
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
            model: None,
            close_after_settled_days: None,
            settle_after_hours: None,
            created_by: "owner".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            node_selector: None,
            stop_when_done: false,
            room: None,
        };

        let cfg = SlackAdapterConfig::from_config(&config);
        assert_eq!(cfg.trigger, MessageTrigger::MentionOnly);
        assert_eq!(cfg.access_policy, SourceAccessPolicy::Open);
    }
}
