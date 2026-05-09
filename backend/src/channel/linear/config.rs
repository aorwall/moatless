//! Typed view of Linear task trigger fields used by the webhook parser.
//!
//! Each `TaskTriggerRoute` is a "listener" with its own filter. Several rules can
//! share a single Linear team UUID (multi-rule support is what migration
//! `0036` enables) — each one is evaluated independently.

use crate::channel::models::TaskTriggerRoute;

/// Default workflow state names whose ENTRY fires task creation.
const DEFAULT_TRIGGER_STATES: &[&str] = &["Todo", "In Progress"];

/// Default workflow state names whose ENTRY closes associated tasks
/// (when `close_task_on_close` is set on the rule).
const DEFAULT_TERMINAL_STATES: &[&str] = &["Done", "Cancelled", "Canceled", "Duplicate"];

/// Extracted Linear-specific knobs from an `TaskTriggerRoute`.
#[derive(Debug, Clone)]
pub struct LinearRuleConfig {
    /// Required labels — issue must carry every label here to trigger.
    /// Empty = no label filter.
    pub require_labels: Vec<String>,
    /// States whose ENTRY fires task creation FOR THIS LISTENER. Per-listener:
    /// each task trigger can configure different trigger states (e.g. one rule
    /// fires on "Todo", another on "In Progress").
    pub trigger_states: Vec<String>,
    /// States whose ENTRY closes tasks owned by this listener (gated by
    /// `close_task_on_close`).
    pub terminal_states: Vec<String>,
    /// Mirror of the rule's scalar `close_task_on_close` column.
    pub close_task_on_close: bool,
}

impl LinearRuleConfig {
    /// Parse from an `TaskTriggerRoute`'s `provider_config` JSONB column plus
    /// the scalar `close_task_on_close` column. We use `provider_config` (not
    /// `event_filter`) because Linear's filter is an OBJECT — `event_filter`
    /// is typed as `Vec<String>` in the public request DTO, suitable for
    /// GitHub's webhook-event-type list but not for Linear's nested knobs.
    pub fn from_rule(rule: &TaskTriggerRoute) -> Self {
        // `provider_config` may be null when no Linear-specific config was
        // supplied — fall back to an empty object so default state lists kick
        // in below.
        let empty = serde_json::Value::Object(serde_json::Map::new());
        let filter = rule.provider_config.as_ref().unwrap_or(&empty);

        let require_labels = string_list(filter, "require_labels").unwrap_or_default();

        let trigger_states = string_list(filter, "trigger_states").unwrap_or_else(|| {
            DEFAULT_TRIGGER_STATES
                .iter()
                .copied()
                .map(String::from)
                .collect()
        });

        let terminal_states = string_list(filter, "terminal_states").unwrap_or_else(|| {
            DEFAULT_TERMINAL_STATES
                .iter()
                .copied()
                .map(String::from)
                .collect()
        });

        Self {
            require_labels,
            trigger_states,
            terminal_states,
            close_task_on_close: rule.close_task_on_close,
        }
    }
}

/// Extract a `Vec<String>` from a JSONB key. Returns `None` when the key is
/// missing or not an array, so callers can substitute defaults vs. an explicit
/// empty list.
fn string_list(value: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    value.get(key).and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::models::RoutingMode;
    use crate::scope::Scope;
    use chrono::Utc;

    fn rule_with(
        provider_config: serde_json::Value,
        close_task_on_close: bool,
    ) -> TaskTriggerRoute {
        TaskTriggerRoute {
            id: "ir_test".into(),
            provider: "linear".into(),
            connection_id: "pc_test".into(),
            source_matcher: "team-uuid".into(),
            source_name: None,
            routing_mode: RoutingMode::ThreadBased,
            event_filter: serde_json::json!([]),
            repository_id: "repo-1".into(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            allow_direct_send: false,
            provider_config: Some(provider_config),
            ongoing_task_id: None,
            close_task_on_close,
            notify_on_success: false,
            active: true,
            created_by: "alice".into(),
            scope: Scope::User,
            team_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn missing_keys_default() {
        let cfg = LinearRuleConfig::from_rule(&rule_with(serde_json::json!({}), true));
        assert!(cfg.require_labels.is_empty());
        assert_eq!(cfg.trigger_states, vec!["Todo", "In Progress"]);
        assert_eq!(
            cfg.terminal_states,
            vec!["Done", "Cancelled", "Canceled", "Duplicate"]
        );
        assert!(cfg.close_task_on_close);
    }

    #[test]
    fn parses_labels_and_states() {
        let cfg = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({
                "require_labels": ["agent", "ai"],
                "trigger_states": ["Todo"],
                "terminal_states": ["Done"],
            }),
            false,
        ));
        assert_eq!(cfg.require_labels, vec!["agent", "ai"]);
        assert_eq!(cfg.trigger_states, vec!["Todo"]);
        assert_eq!(cfg.terminal_states, vec!["Done"]);
        assert!(!cfg.close_task_on_close);
    }

    #[test]
    fn non_array_treated_as_default() {
        // When `trigger_states` is the wrong shape, fall back to defaults
        // rather than silently producing an empty list — matches GitHub PR's
        // rule config behavior.
        let cfg = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({ "trigger_states": "Todo" }),
            true,
        ));
        assert_eq!(cfg.trigger_states, vec!["Todo", "In Progress"]);
    }

    #[test]
    fn empty_array_clears_filter() {
        // Explicit empty list disables the filter — useful when the user
        // wants to accept any state or any label.
        let cfg = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({
                "require_labels": [],
                "trigger_states": [],
                "terminal_states": [],
            }),
            true,
        ));
        assert!(cfg.require_labels.is_empty());
        assert!(cfg.trigger_states.is_empty());
        assert!(cfg.terminal_states.is_empty());
    }

    #[test]
    fn null_provider_config_uses_defaults() {
        let mut rule = rule_with(serde_json::json!({}), true);
        rule.provider_config = None;
        let cfg = LinearRuleConfig::from_rule(&rule);
        assert_eq!(cfg.trigger_states, vec!["Todo", "In Progress"]);
        assert_eq!(
            cfg.terminal_states,
            vec!["Done", "Cancelled", "Canceled", "Duplicate"]
        );
        assert!(cfg.require_labels.is_empty());
    }

    #[test]
    fn two_listeners_distinct_trigger_states() {
        let listener_a = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({"require_labels":["agent"], "trigger_states":["Todo"]}),
            true,
        ));
        let listener_b = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({"require_labels":["bug"], "trigger_states":["In Progress"]}),
            true,
        ));
        assert_eq!(listener_a.trigger_states, vec!["Todo"]);
        assert_eq!(listener_b.trigger_states, vec!["In Progress"]);
        assert_ne!(listener_a.require_labels, listener_b.require_labels);
    }
}
