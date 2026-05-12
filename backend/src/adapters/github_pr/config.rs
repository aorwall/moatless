//! Typed view of GitHub PR task trigger fields used by the webhook parser.

use crate::adapters::models::TaskTriggerRoute;

/// Extracted GitHub-specific knobs from an `TaskTriggerRoute`.
#[derive(Debug, Clone)]
pub struct GithubPrRuleConfig {
    /// Filter which events to process. Empty means all supported events.
    /// Possible values: "pull_request", "issue_comment",
    /// "pull_request_review_comment", "pull_request_review",
    /// "check_run", "check_suite".
    pub events_filter: Vec<String>,
    /// Automatically close the associated task when the PR is merged or closed.
    pub close_task_on_close: bool,
    /// Emit an inbound message when a check_suite completes successfully.
    /// Failures are always reported via check_run regardless of this flag.
    pub notify_on_success: bool,
}

impl GithubPrRuleConfig {
    /// Parse from an `TaskTriggerRoute`'s `event_filter` JSONB column plus
    /// scalar boolean columns.
    pub fn from_rule(rule: &TaskTriggerRoute) -> Self {
        let events_filter = rule
            .event_filter
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            events_filter,
            close_task_on_close: rule.close_task_on_close,
            notify_on_success: rule.notify_on_success,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::Scope;
    use chrono::Utc;

    fn rule_with(
        events_filter: serde_json::Value,
        close_task_on_close: bool,
        notify_on_success: bool,
    ) -> TaskTriggerRoute {
        TaskTriggerRoute {
            id: "ir_test".into(),
            provider: "github".into(),
            connection_id: "pc_test".into(),
            source_matcher: "soaplabs/repo".into(),
            source_name: None,
            routing_mode: crate::adapters::models::RoutingMode::BySubject,
            event_filter: events_filter,
            repository_id: "repo-1".into(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            allow_direct_send: false,
            provider_config: None,
            ongoing_task_id: None,
            close_task_on_close,
            notify_on_success,
            active: true,
            created_by: "alice".into(),
            scope: Scope::User,
            team_id: None,
            owner_user_id: Some("alice".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn parses_event_filter_array() {
        let rule = rule_with(
            serde_json::json!(["pull_request", "issue_comment"]),
            true,
            false,
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert_eq!(cfg.events_filter, vec!["pull_request", "issue_comment"]);
        assert!(cfg.close_task_on_close);
        assert!(!cfg.notify_on_success);
    }

    #[test]
    fn empty_event_filter() {
        let rule = rule_with(serde_json::json!([]), true, false);
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(cfg.events_filter.is_empty());
    }

    #[test]
    fn non_array_event_filter_treated_as_empty() {
        let rule = rule_with(serde_json::json!({}), true, false);
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(cfg.events_filter.is_empty());
    }

    #[test]
    fn close_task_on_close_false_propagates() {
        let rule = rule_with(serde_json::json!([]), false, true);
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(!cfg.close_task_on_close);
        assert!(cfg.notify_on_success);
    }
}
