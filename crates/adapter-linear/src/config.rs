//! Typed view of Linear subscription-loop fields used by the webhook parser.
//!
//! Each `SubscriptionMatch` is a "listener" with its own filter. Several
//! subscriptions can share a single Linear team UUID (multi-listener support is
//! what migration `0036` enables) — each one is evaluated independently.

use moatless_types::loops::SubscriptionMatch;

/// Default workflow state names whose ENTRY fires task creation.
const DEFAULT_TRIGGER_STATES: &[&str] = &["Todo", "In Progress"];

/// Default workflow state TYPES whose ENTRY closes associated tasks (gated by
/// `close_task_on_close`).
///
/// Type names rather than display names, so a workspace that renames "Done" to
/// "Shipped" still closes correctly.
const DEFAULT_TERMINAL_STATES: &[&str] = &["completed", "canceled"];

/// Extracted Linear-specific knobs from a `SubscriptionMatch`.
#[derive(Debug, Clone)]
pub struct LinearRuleConfig {
    /// Required labels — issue must carry every label here to trigger. Empty =
    /// no label filter.
    pub require_labels: Vec<String>,
    /// States whose ENTRY fires task creation FOR THIS LISTENER. Per-listener:
    /// each subscription can configure different trigger states (e.g. one
    /// listener fires on "Todo", another on "In Progress").
    pub trigger_states: Vec<String>,
    /// Whether `trigger_states` came from the config rather than the default.
    /// Only an explicit list gates [`Self::label_trigger_allows_state`].
    pub trigger_states_explicit: bool,
    /// States whose ENTRY closes tasks owned by this listener, gated by
    /// `close_task_on_close`. Each entry matches either the state type (e.g.
    /// `"completed"`) or the state name, case-insensitively; see
    /// [`Self::is_terminal`].
    pub terminal_states: Vec<String>,
    /// Mirror of the subscription's scalar `close_task_on_close` column.
    pub close_task_on_close: bool,
    /// Opt-in: when `true`, an `Issue.update` into a `trigger_states` entry
    /// (satisfying `require_labels`/`project_ids`) spawns a new Moatless task.
    ///
    /// Defaults to `false` so legacy mention/assignment-only subscriptions
    /// with non-empty `trigger_states` don't start auto-creating tasks;
    /// `on: linear` loops opt in explicitly.
    pub create_on_status_entry: bool,
    /// Project allowlist — issue must belong to one of these Linear project
    /// UUIDs to fire status-entry creation. Empty = any project.
    pub project_ids: Vec<String>,
    /// Optional workflow-state NAME to move the issue INTO when a status-entry
    /// task is created (e.g. `"In Progress"`), so the board reflects that the
    /// agent picked the ticket up. `None` leaves the issue's state unchanged.
    pub start_state: Option<String>,
    /// Label names (case-insensitive) whose addition to an issue, or presence
    /// on a newly created one, fires this listener. Empty = no label trigger.
    pub trigger_labels: Vec<String>,
    /// Label names (case-insensitive) that stop every trigger of this listener
    /// for an issue carrying one: mention, delegation, status entry, label.
    pub exclude_labels: Vec<String>,
    /// Whether a Task this listener starts is bound to its issue, and so hears
    /// every comment and state change on it. `false` starts a fresh Task on
    /// each trigger: one that only reads the ticket and moves it on would
    /// otherwise answer every later comment meant for the next Loop's Task.
    pub bind_issue: bool,
}

impl LinearRuleConfig {
    /// Parse from a `SubscriptionMatch`'s `adapter_config` JSONB column plus
    /// the scalar `close_task_on_close` column.
    ///
    /// Uses `adapter_config`, not `event_filter`, because Linear's filter is an
    /// OBJECT while `event_filter` is typed `Vec<String>` for GitHub's
    /// webhook-event-type list.
    pub fn from_rule(rule: &SubscriptionMatch) -> Self {
        // `adapter_config` may be null when no Linear-specific config was
        // supplied — fall back to an empty object so default state lists kick
        // in below.
        let empty = serde_json::Value::Object(serde_json::Map::new());
        let filter = rule.adapter_config.as_ref().unwrap_or(&empty);

        let require_labels = string_list(filter, "require_labels").unwrap_or_default();

        let explicit_trigger_states = string_list(filter, "trigger_states");
        let trigger_states_explicit = explicit_trigger_states.is_some();
        let trigger_states = explicit_trigger_states.unwrap_or_else(|| {
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
            trigger_states_explicit,
            terminal_states,
            close_task_on_close: rule.close_task_on_close,
            create_on_status_entry,
            project_ids,
            start_state,
            trigger_labels: string_list(filter, "trigger_labels").unwrap_or_default(),
            exclude_labels: string_list(filter, "exclude_labels").unwrap_or_default(),
            bind_issue: filter
                .get("bind_issue")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
        }
    }

    /// Whether this listener should spawn a task when the issue enters
    /// `state_name`.
    ///
    /// Requires the opt-in flag and `trigger_states` naming the state, case-
    /// insensitively. A listener with `trigger_labels` needs `trigger_states`
    /// written out: the default list would fire a label Loop on every Todo.
    pub fn creates_on_status(&self, state_name: &str) -> bool {
        self.create_on_status_entry
            && (self.trigger_labels.is_empty() || self.trigger_states_explicit)
            && self
                .trigger_states
                .iter()
                .any(|s| s.eq_ignore_ascii_case(state_name))
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

    /// Whether an issue carrying `labels` is excluded from every trigger.
    pub fn excluded(&self, labels: &[String]) -> bool {
        any_label_matches(&self.exclude_labels, labels)
    }

    /// The first of `added` (label names) that fires this listener's label
    /// trigger.
    pub fn trigger_label_in<'a>(&self, added: &'a [String]) -> Option<&'a str> {
        added
            .iter()
            .find(|l| {
                self.trigger_labels
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(l))
            })
            .map(String::as_str)
    }

    /// Whether a label trigger may fire while the issue sits in `state_name`.
    ///
    /// Gated only by an explicit, non-empty `trigger_states`: the default list
    /// would silently confine every label Loop to Todo and In Progress.
    pub fn label_trigger_allows_state(&self, state_name: &str) -> bool {
        !self.trigger_states_explicit
            || self.trigger_states.is_empty()
            || self
                .trigger_states
                .iter()
                .any(|s| s.eq_ignore_ascii_case(state_name))
    }

    /// Whether a state with this type and name is one of `terminal_states`.
    pub fn is_terminal(&self, state_kind: &str, state_name: &str) -> bool {
        self.terminal_states.iter().any(|s| {
            (!state_kind.is_empty() && s.eq_ignore_ascii_case(state_kind))
                || (!state_name.is_empty() && s.eq_ignore_ascii_case(state_name))
        })
    }
}

fn any_label_matches(configured: &[String], labels: &[String]) -> bool {
    configured
        .iter()
        .any(|c| labels.iter().any(|l| l.eq_ignore_ascii_case(c)))
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

/// Pick the subscription that spawns a task for a mention or assignment: the
/// admitting, non-excluding listener with the highest [`session_scope`] (a tie
/// goes to the oldest), else the first unscoped catch-all, else `rules.first()`.
/// `None` only when `rules` is empty.
pub fn select_spawn_rule<'a>(
    rules: &'a [SubscriptionMatch],
    project_id: Option<&str>,
    labels: &[String],
) -> Option<&'a SubscriptionMatch> {
    // Tier 1 — the narrowest scoped listener that admits this issue.
    let mut best: Option<(usize, &SubscriptionMatch)> = None;
    for rule in rules {
        let cfg = LinearRuleConfig::from_rule(rule);
        let Some(scope) = session_scope(&cfg) else {
            continue;
        };
        if !cfg.project_allowed(project_id) || !cfg.labels_allowed(labels) || cfg.excluded(labels) {
            continue;
        }
        // Strictly greater, so the first listed keeps a tie.
        if best.is_none_or(|(top, _)| scope > top) {
            best = Some((scope, rule));
        }
    }
    if let Some((_, scoped)) = best {
        return Some(scoped);
    }

    // Tier 2 — first unscoped (no session scope) catch-all listener. Labels are
    // not consulted for the catch-all.
    if let Some(fallback) = rules
        .iter()
        .find(|r| session_scope(&LinearRuleConfig::from_rule(r)).is_none())
    {
        return Some(fallback);
    }

    // Tier 3 — no scoped match, no catch-all: oldest listener wins (legacy).
    rules.first()
}

/// A listener's mention-routing scope: one point for a project allowlist and
/// one per required label, `None` for a catch-all. Labels count only on a
/// mention-mode listener: an event-driven one (the factory) already owns a task
/// on its issue, and routing a mention to it would start a second.
fn session_scope(cfg: &LinearRuleConfig) -> Option<usize> {
    let mention_mode = !cfg.create_on_status_entry && cfg.trigger_labels.is_empty();
    let project = usize::from(!cfg.project_ids.is_empty());
    // A project-scoped listener counts its required labels whatever its mode,
    // as Tier 1 always has.
    let labels = if project == 1 || mention_mode {
        cfg.require_labels.len()
    } else {
        0
    };
    match project + labels {
        0 => None,
        scope => Some(scope),
    }
}

/// Whether selecting a spawn listener requires fetching the full issue (project
/// + labels).
///
/// True iff any listener is project-scoped or excludes labels; otherwise
/// neither [`select_spawn_rule`] nor [`LinearRuleConfig::excluded`] reads the
/// issue, so the caller skips the extra API round-trip entirely.
pub fn rules_need_issue_fetch(rules: &[SubscriptionMatch]) -> bool {
    rules.iter().any(|r| {
        let cfg = LinearRuleConfig::from_rule(r);
        session_scope(&cfg).is_some() || !cfg.exclude_labels.is_empty()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::Utc;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::AdapterKind;
    use moatless_types::loops::RoutingMode;

    pub(crate) fn rule_with(
        adapter_config: serde_json::Value,
        close_task_on_close: bool,
    ) -> SubscriptionMatch {
        SubscriptionMatch {
            name: "test-loop".to_string(),
            id: "ir_test".into(),
            adapter_kind: AdapterKind::Linear,
            connection_id: "pc_test".into(),
            source_matcher: "team-uuid".into(),
            source_name: None,
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            workspace_id: "ws-1".into(),
            prompt_template: None,
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
            node_selector: None,
            stop_when_done: None,
            room: None,
            message_buffer_seconds: None,
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
        assert!(!cfg.create_on_status_entry);
        assert!(cfg.project_ids.is_empty());
    }

    #[test]
    fn label_triggers_and_exclusions_match_names_case_insensitively() {
        let cfg = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({
                "trigger_labels": ["Factory"],
                "exclude_labels": ["no-agent"],
            }),
            true,
        ));
        assert_eq!(
            cfg.trigger_label_in(&["bug".into(), "factory".into()]),
            Some("factory")
        );
        assert_eq!(cfg.trigger_label_in(&["bug".into()]), None);
        assert!(cfg.excluded(&["No-Agent".into()]));
        assert!(!cfg.excluded(&["factory".into()]));

        let none = LinearRuleConfig::from_rule(&rule_with(serde_json::json!({}), true));
        assert_eq!(none.trigger_label_in(&["factory".into()]), None);
        assert!(!none.excluded(&["no-agent".into()]));
    }

    #[test]
    fn only_explicit_trigger_states_gate_a_label_trigger() {
        // The default `trigger_states` would confine a label Loop to Todo and
        // In Progress; only a list the Loop wrote may.
        let defaulted = LinearRuleConfig::from_rule(&rule_with(serde_json::json!({}), true));
        assert!(defaulted.label_trigger_allows_state("Backlog"));

        let gated = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({ "trigger_states": ["Todo"] }),
            true,
        ));
        assert!(gated.label_trigger_allows_state("todo"));
        assert!(!gated.label_trigger_allows_state("Backlog"));

        let cleared = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({ "trigger_states": [] }),
            true,
        ));
        assert!(cleared.label_trigger_allows_state("Backlog"));
    }

    #[test]
    fn terminal_states_match_a_state_type_or_a_state_name() {
        let defaulted = LinearRuleConfig::from_rule(&rule_with(serde_json::json!({}), true));
        assert!(defaulted.is_terminal("completed", "Shipped"));
        assert!(!defaulted.is_terminal("started", "In Progress"));

        let named = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({ "terminal_states": ["Done", "Won't Do"] }),
            true,
        ));
        assert!(named.is_terminal("completed", "done"));
        assert!(named.is_terminal("canceled", "Won't Do"));
        assert!(!named.is_terminal("completed", "Shipped"));
        assert!(!named.is_terminal("", ""));
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
        assert!(on.creates_on_status("ready"), "matched case-insensitively");
        assert!(!on.creates_on_status("Todo"));
    }

    #[test]
    fn a_label_listener_creates_on_status_only_in_states_it_names() {
        // Sync turns `create_on_status_entry` on for every Linear Loop, so a
        // label-only Loop must not inherit the default Todo/In Progress.
        let label_only = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({ "create_on_status_entry": true, "trigger_labels": ["factory"] }),
            true,
        ));
        assert!(!label_only.creates_on_status("Todo"));

        let both = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({
                "create_on_status_entry": true,
                "trigger_labels": ["factory"],
                "trigger_states": ["Todo"],
            }),
            true,
        ));
        assert!(both.creates_on_status("Todo"));
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
    fn binds_the_issue_unless_told_not_to() {
        let defaulted = LinearRuleConfig::from_rule(&rule_with(serde_json::json!({}), true));
        assert!(defaulted.bind_issue);
        let unbound = LinearRuleConfig::from_rule(&rule_with(
            serde_json::json!({ "bind_issue": false }),
            true,
        ));
        assert!(!unbound.bind_issue);
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

    /// Parsed `project_ids` of the picked listener — used to assert which
    /// listener the selector returned (the `rule_with` helper gives every
    /// listener the same id, so we distinguish by parsed config rather than
    /// identity).
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

    /// A mention-mode listener scoped by labels alone, e.g. a `spec-issue` Loop.
    fn label_rule(labels: &[&str]) -> SubscriptionMatch {
        rule_with(
            serde_json::json!({ "create_on_status_entry": false, "require_labels": labels }),
            false,
        )
    }

    fn picked_labels(rule: &SubscriptionMatch) -> Vec<String> {
        LinearRuleConfig::from_rule(rule).require_labels
    }

    #[test]
    fn a_label_scoped_mention_listener_beats_the_older_catch_all() {
        let rules = vec![unscoped_rule(), label_rule(&["spec-issue"])];
        let picked = select_spawn_rule(&rules, None, &["spec-issue".to_string()]).unwrap();
        assert_eq!(picked_labels(picked), vec!["spec-issue"]);
    }

    #[test]
    fn a_label_scoped_listener_the_issue_does_not_match_falls_back() {
        let rules = vec![label_rule(&["spec-issue"]), unscoped_rule()];
        let picked = select_spawn_rule(&rules, None, &["Bug".to_string()]).unwrap();
        assert!(picked_labels(picked).is_empty());
    }

    #[test]
    fn the_listener_with_more_constraints_wins() {
        let labels = ["spec-issue".to_string(), "soap-specs".to_string()];
        let rules = vec![
            unscoped_rule(),
            label_rule(&["spec-issue"]),
            label_rule(&["spec-issue", "soap-specs"]),
        ];
        let picked = select_spawn_rule(&rules, None, &labels).unwrap();
        assert_eq!(picked_labels(picked), vec!["spec-issue", "soap-specs"]);
    }

    #[test]
    fn an_event_driven_listener_takes_no_mention_by_its_labels() {
        // The factory gates its own status and label events with
        // `require_labels`; a mention on its ticket must not start a second task
        // from it, so the catch-all keeps the session (and may decline it).
        let factory = rule_with(
            serde_json::json!({
                "create_on_status_entry": true,
                "trigger_labels": ["factory"],
                "require_labels": ["factory"],
            }),
            false,
        );
        let rules = vec![unscoped_rule(), factory];
        let picked = select_spawn_rule(&rules, None, &["factory".to_string()]).unwrap();
        assert!(picked_labels(picked).is_empty());
    }

    #[test]
    fn a_scoped_listener_that_excludes_the_issue_is_passed_over() {
        let excluding = rule_with(
            serde_json::json!({
                "create_on_status_entry": false,
                "require_labels": ["spec-issue"],
                "exclude_labels": ["blocked"],
            }),
            false,
        );
        let rules = vec![excluding, unscoped_rule()];
        let labels = ["spec-issue".to_string(), "blocked".to_string()];
        let picked = select_spawn_rule(&rules, None, &labels).unwrap();
        assert!(picked_labels(picked).is_empty());
    }

    #[test]
    fn rules_need_issue_fetch_true_when_a_listener_is_label_scoped() {
        assert!(rules_need_issue_fetch(&[
            unscoped_rule(),
            label_rule(&["spec-issue"])
        ]));
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
    fn rules_need_issue_fetch_true_when_any_excludes_labels() {
        let excluding = rule_with(serde_json::json!({ "exclude_labels": ["no-agent"] }), false);
        assert!(rules_need_issue_fetch(&[unscoped_rule(), excluding]));
    }

    #[test]
    fn rules_need_issue_fetch_false_when_all_unscoped() {
        // All-unscoped teams skip the get_issue round-trip entirely.
        let rules = vec![unscoped_rule(), unscoped_rule()];
        assert!(!rules_need_issue_fetch(&rules));
        assert!(!rules_need_issue_fetch(&[]));
    }
}
