//! Typed view of Linear subscription-loop fields used by the webhook parser.
//!
//! Each `SubscriptionMatch` is a "listener" with its own filter. Several subscriptions can
//! share a single Linear team UUID (multi-listener support is what migration
//! `0036` enables) — each one is evaluated independently.

use moatless_loops::adapters::models::SubscriptionMatch;

/// Default workflow state names whose ENTRY fires task creation.
const DEFAULT_TRIGGER_STATES: &[&str] = &["Todo", "In Progress"];

/// Default workflow state TYPES whose ENTRY closes associated tasks
/// (when `close_task_on_close` is set on the subscription). These are matched against
/// Linear's stable `state.type` enum, not the user-facing state name — so a
/// workspace that renames "Done" to "Shipped" still closes correctly.
const DEFAULT_TERMINAL_STATES: &[&str] = &["completed", "canceled"];

/// Extracted Linear-specific knobs from a `SubscriptionMatch`.
#[derive(Debug, Clone)]
pub struct LinearRuleConfig {
    /// Required labels — issue must carry every label here to trigger.
    /// Empty = no label filter.
    pub require_labels: Vec<String>,
    /// States whose ENTRY fires task creation FOR THIS LISTENER. Per-listener:
    /// each subscription can configure different trigger states (e.g. one listener
    /// fires on "Todo", another on "In Progress").
    pub trigger_states: Vec<String>,
    /// State TYPES (Linear's `state.type` enum, e.g. `"completed"`,
    /// `"canceled"`) whose ENTRY closes tasks owned by this listener
    /// (gated by `close_task_on_close`). Matched against `state.type` rather
    /// than `state.name` so renamed workspace states still trigger close.
    pub terminal_states: Vec<String>,
    /// Mirror of the subscription's scalar `close_task_on_close` column.
    pub close_task_on_close: bool,
    /// Opt-in: when `true`, an `Issue.update` that moves the issue INTO one of
    /// `trigger_states` (and satisfies `require_labels` / `project_ids`) spawns
    /// a brand-new Moatless task for the issue. Defaults to `false` so the
    /// pre-existing mention/assignment-only Linear subscriptions are unaffected — they
    /// carry the legacy non-empty `trigger_states` default but must NOT start
    /// auto-creating tasks on every status change. `on: linear` loops set this
    /// to `true` explicitly.
    pub create_on_status_entry: bool,
    /// Project allowlist — issue must belong to one of these Linear project
    /// UUIDs to fire status-entry creation. Empty = any project.
    pub project_ids: Vec<String>,
    /// Optional workflow-state NAME to move the issue INTO when a status-entry
    /// task is created (e.g. `"In Progress"`), so the board reflects that the
    /// agent picked the ticket up. `None` leaves the issue's state unchanged.
    pub start_state: Option<String>,
}

impl LinearRuleConfig {
    /// Parse from an `SubscriptionMatch`'s `adapter_config` JSONB column plus
    /// the scalar `close_task_on_close` column. We use `adapter_config` (not
    /// `event_filter`) because Linear's filter is an OBJECT — `event_filter`
    /// is typed as `Vec<String>` in the public request DTO, suitable for
    /// GitHub's webhook-event-type list but not for Linear's nested knobs.
    pub fn from_rule(rule: &SubscriptionMatch) -> Self {
        // `adapter_config` may be null when no Linear-specific config was
        // supplied — fall back to an empty object so default state lists kick
        // in below.
        let empty = serde_json::Value::Object(serde_json::Map::new());
        let filter = rule.adapter_config.as_ref().unwrap_or(&empty);

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

        let create_on_status_entry = filter
            .get("create_on_status_entry")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let project_ids = string_list(filter, "project_ids").unwrap_or_default();

        let start_state = filter
            .get("start_state")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.is_empty());

        Self {
            require_labels,
            trigger_states,
            terminal_states,
            close_task_on_close: rule.close_task_on_close,
            create_on_status_entry,
            project_ids,
            start_state,
        }
    }

    /// Whether this listener should spawn a task when the issue enters
    /// `state_name`. Requires the opt-in flag AND a non-empty `trigger_states`
    /// containing the new state. An explicit empty `trigger_states` therefore
    /// disables status-entry creation even when the flag is set.
    pub fn creates_on_status(&self, state_name: &str) -> bool {
        self.create_on_status_entry && self.trigger_states.iter().any(|s| s == state_name)
    }

    /// Whether `project_id` passes this listener's project allowlist. An empty
    /// allowlist matches any project (including issues with no project).
    pub fn project_allowed(&self, project_id: Option<&str>) -> bool {
        if self.project_ids.is_empty() {
            return true;
        }
        match project_id {
            Some(id) => self.project_ids.iter().any(|p| p == id),
            None => false,
        }
    }

    /// Whether `labels` satisfy this listener's `require_labels` (every
    /// required label must be present). An empty requirement matches anything.
    pub fn labels_allowed(&self, labels: &[String]) -> bool {
        self.require_labels
            .iter()
            .all(|req| labels.iter().any(|l| l == req))
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

/// Pick the subscription that should spawn a task for a mention/assignment, given the
/// issue's `project_id` and `labels`. Several Linear listeners can share one team
/// (e.g. an unscoped `soap-specs` listener plus a project-scoped
/// `vulnerability-fix` listener); team alone can't disambiguate, so we route by
/// project + labels:
///
/// - **Tier 1 — scoped match:** the first listener that is project-scoped
///   (`project_ids` non-empty) AND whose project allowlist admits `project_id`
///   AND whose `require_labels` are all present. A mention on a project that a
///   scoped listener targets routes to that listener.
/// - **Tier 2 — fallback:** the first listener with an EMPTY `project_ids` — the
///   unscoped catch-all. Labels are intentionally NOT consulted here: the
///   catch-all is the default lane for everything not claimed by a scoped listener.
/// - **Tier 3 — legacy:** no scoped match and no unscoped listener → `rules.first()`,
///   preserving the previous "oldest listener wins" behavior for teams that predate
///   project scoping.
///
/// Returns `None` only when `rules` is empty. A failed issue fetch is modeled by
/// the caller as `(None, &[])`, which naturally degrades through Tier 2/3 — no
/// special-casing needed here.
pub fn select_spawn_rule<'a>(
    rules: &'a [SubscriptionMatch],
    project_id: Option<&str>,
    labels: &[String],
) -> Option<&'a SubscriptionMatch> {
    // Tier 1 — first project-scoped listener that admits this issue's project + labels.
    if let Some(scoped) = rules.iter().find(|r| {
        let cfg = LinearRuleConfig::from_rule(r);
        !cfg.project_ids.is_empty() && cfg.project_allowed(project_id) && cfg.labels_allowed(labels)
    }) {
        return Some(scoped);
    }

    // Tier 2 — first unscoped (empty project_ids) catch-all listener. Labels are
    // not consulted for the catch-all.
    if let Some(fallback) = rules
        .iter()
        .find(|r| LinearRuleConfig::from_rule(r).project_ids.is_empty())
    {
        return Some(fallback);
    }

    // Tier 3 — no scoped match, no catch-all: oldest listener wins (legacy).
    rules.first()
}

/// Whether selecting a spawn listener requires fetching the full issue (to learn its
/// project + labels). True iff any listener is project-scoped (`project_ids`
/// non-empty). When every listener is unscoped, [`select_spawn_rule`] can't be
/// influenced by the project/labels, so the caller skips the extra API round-trip
/// entirely — all-unscoped teams pay nothing, preserving the pre-scoping path.
pub fn rules_need_issue_fetch(rules: &[SubscriptionMatch]) -> bool {
    rules
        .iter()
        .any(|r| !LinearRuleConfig::from_rule(r).project_ids.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_loops::adapters::models::RoutingMode;
    use chrono::Utc;
    use moatless_backend_core::scope::Scope;

    fn rule_with(
        adapter_config: serde_json::Value,
        close_task_on_close: bool,
    ) -> SubscriptionMatch {
        SubscriptionMatch {
            id: "ir_test".into(),
            adapter_kind: "linear".into(),
            connection_id: "pc_test".into(),
            source_matcher: "team-uuid".into(),
            source_name: None,
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            repository_id: "repo-1".into(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            skills: Vec::new(),
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            inactive_ttl_days: None,
            allow_direct_send: false,
            adapter_config: Some(adapter_config),
            ongoing_task_id: None,
            close_task_on_close,
            notify_on_success: false,
            active: true,
            created_by: "alice".into(),
            scope: Scope::User,
            owner_user_id: Some("alice".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn missing_keys_default() {
        let cfg = LinearRuleConfig::from_rule(&rule_with(serde_json::json!({}), true));
        assert!(cfg.require_labels.is_empty());
        assert_eq!(cfg.trigger_states, vec!["Todo", "In Progress"]);
        assert_eq!(cfg.terminal_states, vec!["completed", "canceled"]);
        assert!(cfg.close_task_on_close);
        // Status-entry creation is OFF unless explicitly opted in.
        assert!(!cfg.create_on_status_entry);
        assert!(cfg.project_ids.is_empty());
    }

    #[test]
    fn create_on_status_requires_opt_in_flag() {
        // Default config has the legacy non-empty trigger_states but the flag
        // off — it must NOT create on status entry.
        let off = LinearRuleConfig::from_rule(&rule_with(serde_json::json!({}), true));
        assert!(!off.creates_on_status("Todo"));

        let on = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({
                "create_on_status_entry": true,
                "trigger_states": ["Ready"],
            }),
            true,
        ));
        assert!(on.creates_on_status("Ready"));
        assert!(!on.creates_on_status("Todo"));
    }

    #[test]
    fn create_on_status_disabled_by_empty_trigger_states() {
        // Explicit empty trigger_states opts out even with the flag on.
        let cfg = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({
                "create_on_status_entry": true,
                "trigger_states": [],
            }),
            true,
        ));
        assert!(!cfg.creates_on_status("Ready"));
    }

    #[test]
    fn project_allowlist_filters() {
        let cfg = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({ "project_ids": ["proj-a", "proj-b"] }),
            true,
        ));
        assert_eq!(cfg.project_ids, vec!["proj-a", "proj-b"]);
        assert!(cfg.project_allowed(Some("proj-a")));
        assert!(!cfg.project_allowed(Some("proj-z")));
        // No project on the issue → rejected when an allowlist is set.
        assert!(!cfg.project_allowed(None));

        // Empty allowlist accepts anything, including project-less issues.
        let any = LinearRuleConfig::from_rule(&rule_with(serde_json::json!({}), true));
        assert!(any.project_allowed(Some("proj-a")));
        assert!(any.project_allowed(None));
    }

    #[test]
    fn parses_start_state() {
        let cfg = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({ "start_state": "In Progress" }),
            true,
        ));
        assert_eq!(cfg.start_state.as_deref(), Some("In Progress"));

        // Absent or empty → no transition.
        let none = LinearRuleConfig::from_rule(&rule_with(serde_json::json!({}), true));
        assert_eq!(none.start_state, None);
        let empty =
            LinearRuleConfig::from_rule(&rule_with(serde_json::json!({ "start_state": "" }), true));
        assert_eq!(empty.start_state, None);
    }

    #[test]
    fn labels_allowed_requires_all() {
        let cfg = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({ "require_labels": ["agent", "ready"] }),
            true,
        ));
        assert!(cfg.labels_allowed(&["agent".into(), "ready".into(), "bug".into()]));
        assert!(!cfg.labels_allowed(&["agent".into()]));
        // No requirements → anything passes.
        let any = LinearRuleConfig::from_rule(&rule_with(serde_json::json!({}), true));
        assert!(any.labels_allowed(&[]));
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
    fn null_adapter_config_uses_defaults() {
        let mut rule = rule_with(serde_json::json!({}), true);
        rule.adapter_config = None;
        let cfg = LinearRuleConfig::from_rule(&rule);
        assert_eq!(cfg.trigger_states, vec!["Todo", "In Progress"]);
        assert_eq!(cfg.terminal_states, vec!["completed", "canceled"]);
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

    // ---- select_spawn_rule / rules_need_issue_fetch ----------------------

    /// A project-scoped listener (`project_ids = ["vuln"]`).
    fn scoped_rule() -> SubscriptionMatch {
        rule_with(serde_json::json!({ "project_ids": ["vuln"] }), false)
    }

    /// The unscoped catch-all (empty `project_ids`), e.g. soap-specs.
    fn unscoped_rule() -> SubscriptionMatch {
        rule_with(serde_json::json!({}), false)
    }

    /// Parsed `project_ids` of the picked listener — used to assert which listener the
    /// selector returned (the `rule_with` helper gives every listener the same id,
    /// so we distinguish by parsed config rather than identity).
    fn picked_projects(rule: &SubscriptionMatch) -> Vec<String> {
        LinearRuleConfig::from_rule(rule).project_ids
    }

    #[test]
    fn select_spawn_rule_scoped_wins() {
        // Unscoped catch-all listed first, scoped listener second — the scoped listener
        // must still win when the issue's project matches it.
        let rules = vec![unscoped_rule(), scoped_rule()];
        let picked = select_spawn_rule(&rules, Some("vuln"), &[]).unwrap();
        assert_eq!(picked_projects(picked), vec!["vuln"]);
    }

    #[test]
    fn select_spawn_rule_project_mismatch_falls_back() {
        // Scoped listener targets "vuln"; the issue is in another project → the
        // unscoped catch-all wins.
        let rules = vec![scoped_rule(), unscoped_rule()];
        let picked = select_spawn_rule(&rules, Some("other"), &[]).unwrap();
        assert!(picked_projects(picked).is_empty());
    }

    #[test]
    fn select_spawn_rule_label_mismatch_falls_back() {
        // Scoped listener requires a label the issue lacks → falls back to the
        // unscoped catch-all (whose labels are not consulted).
        let scoped = rule_with(
            serde_json::json!({ "project_ids": ["vuln"], "require_labels": ["security"] }),
            false,
        );
        let rules = vec![scoped, unscoped_rule()];
        let picked = select_spawn_rule(&rules, Some("vuln"), &[]).unwrap();
        assert!(picked_projects(picked).is_empty());
    }

    #[test]
    fn select_spawn_rule_fetch_failure_falls_back() {
        // A failed issue fetch is modeled as (None, &[]) — the scoped listener can't
        // match a project-less issue, so the catch-all wins.
        let rules = vec![scoped_rule(), unscoped_rule()];
        let picked = select_spawn_rule(&rules, None, &[]).unwrap();
        assert!(picked_projects(picked).is_empty());
    }

    #[test]
    fn select_spawn_rule_no_fallback_uses_oldest() {
        // Two scoped listeners, neither matching the issue's project, and no
        // unscoped catch-all → Tier 3: oldest (first) listener wins.
        let first = rule_with(serde_json::json!({ "project_ids": ["a"] }), false);
        let second = rule_with(serde_json::json!({ "project_ids": ["b"] }), false);
        let rules = vec![first, second];
        let picked = select_spawn_rule(&rules, Some("other"), &[]).unwrap();
        assert_eq!(picked_projects(picked), vec!["a"]);
    }

    #[test]
    fn select_spawn_rule_first_of_several_scoped_wins() {
        // Two scoped listeners both admit the issue — the first listed wins. They
        // share a project but carry distinct require_labels (both satisfied), so
        // the picked listener is identifiable by its labels.
        let first = rule_with(
            serde_json::json!({ "project_ids": ["vuln"], "require_labels": ["a"] }),
            false,
        );
        let second = rule_with(
            serde_json::json!({ "project_ids": ["vuln"], "require_labels": ["b"] }),
            false,
        );
        let rules = vec![first, second];
        let picked =
            select_spawn_rule(&rules, Some("vuln"), &["a".to_string(), "b".to_string()]).unwrap();
        assert_eq!(
            LinearRuleConfig::from_rule(picked).require_labels,
            vec!["a"]
        );
    }

    #[test]
    fn select_spawn_rule_empty_rules_is_none() {
        assert!(select_spawn_rule(&[], Some("vuln"), &[]).is_none());
    }

    #[test]
    fn rules_need_issue_fetch_true_when_any_scoped() {
        let rules = vec![unscoped_rule(), scoped_rule()];
        assert!(rules_need_issue_fetch(&rules));
    }

    #[test]
    fn rules_need_issue_fetch_false_when_all_unscoped() {
        // All-unscoped teams skip the get_issue round-trip entirely.
        let rules = vec![unscoped_rule(), unscoped_rule()];
        assert!(!rules_need_issue_fetch(&rules));
        assert!(!rules_need_issue_fetch(&[]));
    }
}
