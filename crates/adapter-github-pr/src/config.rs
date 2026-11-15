//! Typed view of GitHub PR subscription-loop fields used by the webhook parser.

use moatless_types::adapter::InboundEvent;
use moatless_types::loops::SubscriptionMatch;

/// Senders ignored by default when `adapter_config.ignored_senders` is not set.
///
/// `linear-code[bot]` posts `<!-- linear-linkback -->` metadata comments that
/// no bound task wants delivered.
const DEFAULT_IGNORED_SENDERS: &[&str] = &["linear-code[bot]"];

/// Derived event name for a base-branch conflict, so a loop can include or
/// ignore it like any other event.
///
/// GitHub has no such webhook event; this adapter derives it from a change in
/// the PR's mergeable state on any PR-bearing event.
pub(super) const MERGE_CONFLICT: &str = "merge_conflict";

/// A base-branch CI run carries no PR reference, so the post-merge notification
/// is likewise derived rather than received. Named for the same reason.
pub(super) const POST_MERGE: &str = "post_merge";

/// Derived event name for a merged PR, so a loop can include or ignore it like
/// any other event.
///
/// GitHub has no `merged` action: a merge arrives as `pull_request` `closed`
/// carrying `merged: true`.
pub(super) const PR_MERGED: &str = "pr_merged";

/// Derived event name for a `check_run` matching a `create_on: check_run`
/// loop's trigger, so a loop can ignore it without dropping failed-check
/// notices.
pub(super) const CHECK_RUN_TRIGGER: &str = "check_run_trigger";

/// Derived event name for a body-less `pull_request_review` verdict. Only
/// `ignored_events` reads it, deliberately: a loop whose include list names
/// `pull_request_review` keeps receiving verdicts.
pub(super) const REVIEW_VERDICT: &str = "review_verdict";

/// Every event name a loop's include/ignore list can name: the GitHub webhook
/// event types this adapter handles, plus the derived signals above.
///
/// `workflow_run` produces no message of its own — it is only ever the carrier
/// for [`POST_MERGE`] — but naming it here is not a typo.
pub(super) const KNOWN_EVENTS: &[&str] = &[
    "pull_request",
    "issue_comment",
    "pull_request_review_comment",
    "pull_request_review",
    "check_run",
    "check_suite",
    "workflow_run",
    MERGE_CONFLICT,
    POST_MERGE,
    PR_MERGED,
    CHECK_RUN_TRIGGER,
    REVIEW_VERDICT,
];

/// The [`KNOWN_EVENTS`] name `name` as an inbound event kind.
///
/// Only comments and reviews arrive in bursts: one review posts a webhook per
/// comment within seconds. Every other event is one notice, held only to join
/// a burst already waiting on its Binding.
pub(super) fn inbound_event(name: &str) -> InboundEvent {
    InboundEvent {
        name: name.to_string(),
        arrives_in_bursts: matches!(
            name,
            "issue_comment"
                | "pull_request_review_comment"
                | "pull_request_review"
                | REVIEW_VERDICT
        ),
    }
}

/// One `adapter_config.ignored_senders` entry: either a bare login, or
/// `{login, events}` naming the [`KNOWN_EVENTS`] it drops from that sender.
///
/// A scoped entry drops only the named events' own messages, so a CI bot's
/// comments can be ignored while its pushes still wake a review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IgnoredSender {
    pub login: String,
    /// Empty drops every event, derived signals included.
    pub events: Vec<String>,
}

/// What creates a Task under a loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum CreateOn {
    /// A pull request opening or reopening.
    #[default]
    PrOpened,
    /// A completed check run matching [`CheckRunTriggerConfig`]; an opening
    /// creates nothing.
    CheckRun,
}

/// `adapter_config.check_run`: which completed check runs trigger the loop.
/// A check run must match all three lists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct CheckRunTriggerConfig {
    /// Check run names. Empty matches none.
    pub names: Vec<String>,
    /// GitHub App ids whose check runs are trusted. Empty trusts none, so a
    /// loop that forgot the key creates no Task from anyone's check run.
    pub app_ids: Vec<u64>,
    /// Accepted conclusions. `None` (the key absent) accepts any.
    pub conclusions: Option<Vec<String>>,
}

impl CheckRunTriggerConfig {
    /// Whether a completed check run with these facts triggers the loop.
    pub(super) fn matches(&self, name: &str, app_id: Option<u64>, conclusion: &str) -> bool {
        self.names.iter().any(|n| n == name)
            && app_id.is_some_and(|id| self.app_ids.contains(&id))
            && self
                .conclusions
                .as_ref()
                .is_none_or(|accepted| accepted.iter().any(|c| c == conclusion))
    }
}

/// Extracted GitHub-specific knobs from an `SubscriptionMatch`.
#[derive(Debug, Clone)]
pub(super) struct GithubPrRuleConfig {
    /// Events the loop's own Tasks receive, named from [`KNOWN_EVENTS`]. Empty
    /// means all of them; see [`Self::mutes`].
    pub events_filter: Vec<String>,
    /// Events never processed, named from [`KNOWN_EVENTS`]. Applied after
    /// `events_filter`, so a name here wins over the same name there. Lets a
    /// loop drop one event without having to enumerate every event it keeps.
    pub ignored_events: Vec<String>,
    /// Automatically close the associated task when the PR is merged or closed.
    pub close_task_on_close: bool,
    /// Emit an inbound message when a `check_suite` completes successfully.
    /// Failures are always reported via `check_run` regardless of this flag.
    pub notify_on_success: bool,
    /// Senders whose events the loop's own Tasks never receive
    /// (case-insensitive against `sender.login`). An entry without `events`
    /// also stops that sender opening or closing the loop's Tasks; see
    /// [`IgnoredSender`].
    pub ignored_senders: Vec<IgnoredSender>,
    /// Regex patterns matched case-insensitively against the PR title.
    ///
    /// A PR whose title matches any pattern never creates a review task
    /// (applies to `pull_request` opened/reopened only). Empty/absent = no
    /// filtering.
    pub ignored_title_patterns: Vec<String>,
    /// `adapter_config.create_on`: `pr_opened` (the default) or `check_run`.
    pub create_on: CreateOn,
    pub check_run: CheckRunTriggerConfig,
    /// `provision.workspace_template`: the Workspace whose settings a new
    /// per-repository Workspace copies under `workspace: auto`. `None` never
    /// creates one.
    pub workspace_template: Option<String>,
}

impl GithubPrRuleConfig {
    /// Parse from an `SubscriptionMatch`'s `event_filter` and `adapter_config`
    /// JSONB columns plus scalar boolean columns.
    pub(super) fn from_rule(rule: &SubscriptionMatch) -> Self {
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
            ignored_events: string_array(rule.adapter_config.as_ref(), "ignored_events"),
            ignored_senders: ignored_senders(rule.adapter_config.as_ref()),
            ignored_title_patterns: ignored_title_patterns(rule.adapter_config.as_ref()),
            create_on: create_on(rule.adapter_config.as_ref()),
            check_run: check_run(rule.adapter_config.as_ref()),
            workspace_template: rule
                .adapter_config
                .as_ref()
                .and_then(|pc| pc.pointer("/provision/workspace_template"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        }
    }

    /// Names in either list that are not in [`KNOWN_EVENTS`]. Such a name is
    /// silently inert — it neither includes nor ignores anything — so callers
    /// surface it rather than let a typo read as a working filter.
    pub(super) fn unknown_event_names(&self) -> Vec<&str> {
        self.events_filter
            .iter()
            .chain(&self.ignored_events)
            .chain(self.ignored_senders.iter().flat_map(|s| &s.events))
            .map(String::as_str)
            .filter(|name| {
                !KNOWN_EVENTS
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(name))
            })
            .collect()
    }

    /// True when `event` is in `ignored_events`, or when `events_filter` is set
    /// and does not name it. An empty `events_filter` means "all events". Both
    /// lists are compared case-insensitively.
    pub(super) fn event_filtered_out(&self, event: &str) -> bool {
        if self.explicitly_ignores(event) {
            return true;
        }
        !self.events_filter.is_empty()
            && !self
                .events_filter
                .iter()
                .any(|e| e.eq_ignore_ascii_case(event))
    }

    /// True when `event` is in `ignored_events`, whatever `events_filter`
    /// holds. For a derived event that refines one the include list names.
    pub(super) fn explicitly_ignores(&self, event: &str) -> bool {
        self.ignored_events
            .iter()
            .any(|e| e.eq_ignore_ascii_case(event))
    }

    /// True when an unscoped `ignored_senders` entry names `sender` (a
    /// `sender.login`), so the whole delivery is dropped. An empty sender
    /// never matches.
    pub(super) fn is_ignored_sender(&self, sender: &str) -> bool {
        self.sender_entries(sender).any(|s| s.events.is_empty())
    }

    /// True when an `ignored_senders` entry for `sender` drops `event`'s own
    /// message: an unscoped entry, or a scoped one that names `event`.
    pub(super) fn ignores_sender_event(&self, sender: &str, event: &str) -> bool {
        self.sender_entries(sender)
            .any(|s| s.events.is_empty() || s.events.iter().any(|e| e.eq_ignore_ascii_case(event)))
    }

    fn sender_entries<'a>(&'a self, sender: &'a str) -> impl Iterator<Item = &'a IgnoredSender> {
        self.ignored_senders
            .iter()
            .filter(move |s| !sender.is_empty() && s.login.eq_ignore_ascii_case(sender))
    }

    /// Whether this loop's filters keep `event` from the Tasks the loop opened.
    ///
    /// Never asked for any other Task: a loop's filters shape its own Tasks'
    /// inbox, so applying them to a Task `moat gh pr create` bound would hide
    /// that Task's CI failures from it.
    pub(super) fn mutes(&self, event: &FilterKey) -> bool {
        let event_muted = if event.event.is_empty() {
            false
        } else if event.event == REVIEW_VERDICT {
            self.explicitly_ignores(REVIEW_VERDICT)
                || self.event_filtered_out("pull_request_review")
        } else {
            self.event_filtered_out(&event.event)
        };
        // A verdict is a `pull_request_review`, so a scoped entry naming that
        // event silences it too.
        let sender_event = if event.event == REVIEW_VERDICT {
            "pull_request_review"
        } else {
            &event.event
        };
        event_muted || self.ignores_sender_event(&event.sender, sender_event)
    }
}

/// What a loop's event and sender filters are matched against, carried with a
/// delivery so routing can apply each bound Task's own loop's filters.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FilterKey {
    /// A [`KNOWN_EVENTS`] name; empty for a delivery admitted before this
    /// field existed, which no loop's filter mutes.
    #[serde(default)]
    pub(crate) event: String,
    /// The webhook's `sender.login`.
    #[serde(default)]
    pub(crate) sender: String,
    /// A submitted review in state `approved`, with a body or without.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) approval: bool,
}

impl FilterKey {
    pub(crate) fn new(event: &str, sender: &str) -> Self {
        Self {
            event: event.to_string(),
            sender: sender.to_string(),
            approval: false,
        }
    }

    /// Whether a Task no loop opened is spared this event: an approval or a
    /// passing `check_suite` asks nothing of the Task that wrote the change.
    pub(super) fn mutes_unlooped_task(&self) -> bool {
        self.approval || self.event == "check_suite"
    }

    /// Whether this key is the one a record admitted without it reads as.
    pub(crate) fn is_unset(&self) -> bool {
        *self == Self::default()
    }

    /// Whether `sender` is one no Task hears from, whatever loop it came from.
    pub(super) fn sender_is_default_ignored(&self) -> bool {
        DEFAULT_IGNORED_SENDERS
            .iter()
            .any(|s| s.eq_ignore_ascii_case(&self.sender))
    }
}

/// Entries of `ignored_events` that are not strings, and of `ignored_senders`
/// that are neither a login nor an object with a string `login`.
///
/// Each one filters nothing, so the caller warns rather than let it read as a
/// working filter.
pub(super) fn inert_filter_entries(
    adapter_config: Option<&serde_json::Value>,
) -> Vec<&serde_json::Value> {
    let entries = |key: &str| {
        adapter_config
            .and_then(|config| config.get(key))
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
    };
    let sender_is_inert = |entry: &serde_json::Value| {
        !entry.is_string()
            && entry
                .get("login")
                .and_then(serde_json::Value::as_str)
                .is_none()
    };
    entries("ignored_events")
        .filter(|entry| !entry.is_string())
        .chain(entries("ignored_senders").filter(|entry| sender_is_inert(entry)))
        .collect()
}

/// Parse `ignored_senders` from a `adapter_config` JSONB value: absent key,
/// null config, or non-array value fall back to `DEFAULT_IGNORED_SENDERS`; an
/// explicit array replaces the default verbatim (`[]` opts out). An entry that
/// is neither a string nor an object with a string `login` is skipped.
fn ignored_senders(adapter_config: Option<&serde_json::Value>) -> Vec<IgnoredSender> {
    let unscoped = |login: &str| IgnoredSender {
        login: login.to_string(),
        events: Vec::new(),
    };
    let Some(entries) = adapter_config
        .and_then(|pc| pc.get("ignored_senders"))
        .and_then(|v| v.as_array())
    else {
        return DEFAULT_IGNORED_SENDERS
            .iter()
            .map(|l| unscoped(l))
            .collect();
    };
    entries
        .iter()
        .filter_map(|entry| match entry {
            serde_json::Value::String(login) => Some(unscoped(login)),
            serde_json::Value::Object(_) => Some(IgnoredSender {
                login: entry.get("login")?.as_str()?.to_string(),
                events: string_array(Some(entry), "events"),
            }),
            _ => None,
        })
        .collect()
}

/// Parse an opt-in string array out of a `adapter_config` JSONB value. An
/// absent key, null config, or non-array value all yield an empty list.
fn string_array(adapter_config: Option<&serde_json::Value>, key: &str) -> Vec<String> {
    adapter_config
        .and_then(|pc| pc.get(key))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `ignored_title_patterns` from a `adapter_config` JSONB value.
///
/// Unlike `ignored_senders`, this is opt-in: absent key, null config, or
/// non-array value all yield an empty list (no filtering).
pub(super) fn ignored_title_patterns(adapter_config: Option<&serde_json::Value>) -> Vec<String> {
    string_array(adapter_config, "ignored_title_patterns")
}

/// Parse `create_on`. Anything but `check_run` keeps the default: sync
/// refuses the wildcard source and `workspace: auto` on such a loop, so a typo
/// cannot widen what creates Tasks.
fn create_on(adapter_config: Option<&serde_json::Value>) -> CreateOn {
    match adapter_config
        .and_then(|pc| pc.get("create_on"))
        .and_then(serde_json::Value::as_str)
    {
        Some("check_run") => CreateOn::CheckRun,
        _ => CreateOn::PrOpened,
    }
}

fn check_run(adapter_config: Option<&serde_json::Value>) -> CheckRunTriggerConfig {
    let Some(section) = adapter_config.and_then(|pc| pc.get("check_run")) else {
        return CheckRunTriggerConfig::default();
    };
    let strings = |key: &str| {
        section
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
    };
    CheckRunTriggerConfig {
        names: strings("names").unwrap_or_default(),
        app_ids: section
            .get("app_ids")
            .and_then(serde_json::Value::as_array)
            .map(|ids| ids.iter().filter_map(serde_json::Value::as_u64).collect())
            .unwrap_or_default(),
        conclusions: strings("conclusions"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::AdapterKind;

    /// Comments and reviews are held for a burst; a notice such as a passed
    /// post-merge run is not.
    #[test]
    fn only_comments_and_reviews_arrive_in_bursts() {
        for name in [
            "issue_comment",
            "pull_request_review_comment",
            "pull_request_review",
            REVIEW_VERDICT,
        ] {
            assert!(inbound_event(name).arrives_in_bursts, "{name}");
        }
        for name in [
            "pull_request",
            "check_run",
            "check_suite",
            MERGE_CONFLICT,
            POST_MERGE,
            PR_MERGED,
            CHECK_RUN_TRIGGER,
        ] {
            assert!(!inbound_event(name).arrives_in_bursts, "{name}");
        }
    }

    fn rule_with(
        events_filter: serde_json::Value,
        close_task_on_close: bool,
        notify_on_success: bool,
    ) -> SubscriptionMatch {
        rule_with_adapter_config(events_filter, close_task_on_close, notify_on_success, None)
    }

    fn rule_with_adapter_config(
        events_filter: serde_json::Value,
        close_task_on_close: bool,
        notify_on_success: bool,
        adapter_config: Option<serde_json::Value>,
    ) -> SubscriptionMatch {
        SubscriptionMatch {
            name: "test-loop".to_string(),
            id: "ir_test".into(),
            adapter_kind: AdapterKind::Github,
            connection_id: "pc_test".into(),
            source_matcher: "soaplabs/repo".into(),
            source_name: None,
            routing_mode: moatless_types::loops::RoutingMode::BySubject,
            event_filter: events_filter,
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
            adapter_config,
            ongoing_task_id: None,
            close_task_on_close,
            notify_on_success,
            active: true,
            created_by: "alice".into(),
            scope: Scope::User,
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

    #[test]
    fn null_adapter_config_uses_default_ignored_senders() {
        let rule = rule_with(serde_json::json!([]), true, false);
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert_eq!(logins(&cfg), vec!["linear-code[bot]"]);
    }

    fn logins(cfg: &GithubPrRuleConfig) -> Vec<&str> {
        cfg.ignored_senders
            .iter()
            .map(|s| s.login.as_str())
            .collect()
    }

    #[test]
    fn empty_ignored_senders_opts_out_of_default() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_senders": []})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(cfg.ignored_senders.is_empty());
    }

    #[test]
    fn explicit_ignored_senders_replace_default() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_senders": ["dependabot[bot]"]})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert_eq!(logins(&cfg), vec!["dependabot[bot]"]);
    }

    #[test]
    fn non_array_ignored_senders_falls_back_to_default() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_senders": "linear-code[bot]"})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert_eq!(logins(&cfg), vec!["linear-code[bot]"]);
    }

    #[test]
    fn absent_ignored_title_patterns_is_empty() {
        let rule = rule_with(serde_json::json!([]), true, false);
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(cfg.ignored_title_patterns.is_empty());
    }

    #[test]
    fn explicit_ignored_title_patterns_parsed() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_title_patterns": ["^chore\\(deps", "^deps[:(]"]})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert_eq!(
            cfg.ignored_title_patterns,
            vec!["^chore\\(deps", "^deps[:(]"]
        );
    }

    #[test]
    fn non_array_ignored_title_patterns_is_empty() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_title_patterns": "^chore"})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(cfg.ignored_title_patterns.is_empty());
    }

    #[test]
    fn empty_event_filter_never_filters_out() {
        let rule = rule_with(serde_json::json!([]), true, false);
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(!cfg.event_filtered_out("workflow_run"));
        assert!(!cfg.event_filtered_out("pull_request"));
    }

    #[test]
    fn event_filter_excludes_events_not_listed() {
        let rule = rule_with(serde_json::json!(["pull_request"]), true, false);
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(!cfg.event_filtered_out("pull_request"));
        assert!(cfg.event_filtered_out("workflow_run"));
        assert!(cfg.event_filtered_out("issue_comment"));
    }

    #[test]
    fn derived_signals_are_included_by_default() {
        let rule = rule_with(serde_json::json!([]), true, false);
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        for event in KNOWN_EVENTS {
            assert!(!cfg.event_filtered_out(event), "{event}");
        }
    }

    #[test]
    fn ignored_events_drops_only_the_named_event() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_events": [MERGE_CONFLICT]})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(cfg.event_filtered_out(MERGE_CONFLICT));
        // The events it is derived from keep flowing.
        assert!(!cfg.event_filtered_out("pull_request"));
        assert!(!cfg.event_filtered_out("issue_comment"));
        assert!(!cfg.event_filtered_out(POST_MERGE));
        assert!(!cfg.event_filtered_out(PR_MERGED));
    }

    #[test]
    fn ignoring_the_merged_signal_keeps_the_pull_request_event() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_events": [PR_MERGED]})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(cfg.event_filtered_out(PR_MERGED));
        // `pull_request` is the carrier, so it keeps its own message and its
        // other derived signals.
        assert!(!cfg.event_filtered_out("pull_request"));
        assert!(!cfg.event_filtered_out(MERGE_CONFLICT));
        assert!(cfg.unknown_event_names().is_empty());
    }

    #[test]
    fn ignored_events_wins_over_the_include_list() {
        let rule = rule_with_adapter_config(
            serde_json::json!(["pull_request", MERGE_CONFLICT]),
            true,
            false,
            Some(serde_json::json!({"ignored_events": [MERGE_CONFLICT]})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(cfg.event_filtered_out(MERGE_CONFLICT));
        assert!(!cfg.event_filtered_out("pull_request"));
    }

    #[test]
    fn a_review_only_loop_keeps_the_diff_and_discussion_events() {
        // The shape a PR-review loop runs: it reads the diff and the
        // conversation, and acts on neither CI nor mergeability.
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({
                "ignored_events": [MERGE_CONFLICT, POST_MERGE, "check_run", "check_suite"]
            })),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);

        for dropped in [MERGE_CONFLICT, POST_MERGE, "check_run", "check_suite"] {
            assert!(cfg.event_filtered_out(dropped), "{dropped}");
        }
        for kept in [
            "pull_request",
            "issue_comment",
            "pull_request_review_comment",
            "pull_request_review",
        ] {
            assert!(!cfg.event_filtered_out(kept), "{kept}");
        }
        assert!(cfg.unknown_event_names().is_empty());
    }

    #[test]
    fn unknown_event_names_are_reported_from_both_lists() {
        let rule = rule_with_adapter_config(
            serde_json::json!(["pull_request", "pull_requests"]),
            true,
            false,
            Some(serde_json::json!({"ignored_events": ["merge_conflicts", "MERGE_CONFLICT"]})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        // The plural typos are inert; the correctly-named (if oddly-cased) one is not.
        assert_eq!(
            cfg.unknown_event_names(),
            vec!["pull_requests", "merge_conflicts"]
        );
    }

    #[test]
    fn event_names_match_case_insensitively() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_events": ["Merge_Conflict"]})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(cfg.event_filtered_out(MERGE_CONFLICT));
    }

    #[test]
    fn is_ignored_sender_matches_case_insensitively() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_senders": ["Dependabot[bot]"]})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert!(cfg.is_ignored_sender("dependabot[bot]"));
        assert!(cfg.is_ignored_sender("DEPENDABOT[BOT]"));
        assert!(!cfg.is_ignored_sender("octocat"));
    }

    #[test]
    fn scoped_ignored_sender_drops_only_the_named_events() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_senders": [
                {"login": "GitHub-Actions[bot]", "events": ["issue_comment"]},
                "renovate[bot]",
                {"events": ["issue_comment"]},
                7
            ]})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert_eq!(logins(&cfg), vec!["GitHub-Actions[bot]", "renovate[bot]"]);
        assert!(!cfg.is_ignored_sender("github-actions[bot]"));
        assert!(cfg.ignores_sender_event("github-actions[bot]", "Issue_Comment"));
        assert!(!cfg.ignores_sender_event("github-actions[bot]", "pull_request"));
        assert!(cfg.is_ignored_sender("renovate[bot]"));
        assert!(cfg.ignores_sender_event("renovate[bot]", "pull_request"));
        assert!(!cfg.ignores_sender_event("", "issue_comment"));
    }

    #[test]
    fn scoped_ignored_sender_events_are_checked_for_typos() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"ignored_senders": [
                {"login": "github-actions[bot]", "events": ["issue_comments"]}
            ]})),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert_eq!(cfg.unknown_event_names(), vec!["issue_comments"]);
    }

    #[test]
    fn is_ignored_sender_empty_login_never_matches() {
        let rule = rule_with(serde_json::json!([]), true, false);
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        // default ignored senders is non-empty, but an empty login must not match.
        assert!(!cfg.is_ignored_sender(""));
    }

    #[test]
    fn create_on_defaults_to_pr_opened() {
        let rule = rule_with(serde_json::json!([]), true, false);
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert_eq!(cfg.create_on, CreateOn::PrOpened);
        assert_eq!(cfg.check_run, CheckRunTriggerConfig::default());
        assert!(cfg.workspace_template.is_none());
    }

    #[test]
    fn parses_the_check_run_section_and_provision() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({
                "create_on": "check_run",
                "check_run": {
                    "names": ["gate"],
                    "app_ids": [5_421_124],
                    "conclusions": ["neutral"],
                },
                "provision": {"workspace_template": "review-template"},
            })),
        );
        let cfg = GithubPrRuleConfig::from_rule(&rule);
        assert_eq!(cfg.create_on, CreateOn::CheckRun);
        assert_eq!(
            cfg.check_run,
            CheckRunTriggerConfig {
                names: vec!["gate".to_string()],
                app_ids: vec![5_421_124],
                conclusions: Some(vec!["neutral".to_string()]),
            }
        );
        assert_eq!(cfg.workspace_template.as_deref(), Some("review-template"));
    }

    #[test]
    fn an_unknown_create_on_keeps_the_default() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            true,
            false,
            Some(serde_json::json!({"create_on": "review_request"})),
        );
        assert_eq!(
            GithubPrRuleConfig::from_rule(&rule).create_on,
            CreateOn::PrOpened
        );
    }

    #[test]
    fn a_trigger_needs_its_name_app_and_conclusion() {
        let trigger = CheckRunTriggerConfig {
            names: vec!["gate".to_string()],
            app_ids: vec![7],
            conclusions: Some(vec!["neutral".to_string()]),
        };
        assert!(trigger.matches("gate", Some(7), "neutral"));
        assert!(!trigger.matches("CI", Some(7), "neutral"));
        assert!(!trigger.matches("gate", Some(8), "neutral"));
        assert!(!trigger.matches("gate", None, "neutral"));
        assert!(!trigger.matches("gate", Some(7), "failure"));
        let any_conclusion = CheckRunTriggerConfig {
            conclusions: None,
            ..trigger
        };
        assert!(any_conclusion.matches("gate", Some(7), "failure"));
    }

    /// A sender only the loop ignores mutes the loop's own Tasks, a scoped
    /// entry only for the events it names, a verdict as a review; the check is
    /// case-insensitive, like the list it reads.
    #[test]
    fn a_loop_ignored_sender_mutes_its_tasks() {
        let rule = rule_with_adapter_config(
            serde_json::json!([]),
            false,
            false,
            Some(serde_json::json!({"ignored_senders": [
                "renovate[bot]",
                {"login": "github-actions[bot]", "events": ["issue_comment", "pull_request_review"]},
            ]})),
        );
        let config = GithubPrRuleConfig::from_rule(&rule);
        assert!(config.mutes(&FilterKey::new("pull_request", "Renovate[bot]")));
        assert!(config.mutes(&FilterKey::new("issue_comment", "GitHub-Actions[bot]")));
        assert!(config.mutes(&FilterKey::new(REVIEW_VERDICT, "github-actions[bot]")));
        assert!(!config.mutes(&FilterKey::new("pull_request", "github-actions[bot]")));
        assert!(!config.mutes(&FilterKey::new("issue_comment", "alice")));
    }

    /// A delivery admitted before `FilterKey` existed carries an empty event,
    /// which an include list must not mute.
    #[test]
    fn an_empty_filter_key_mutes_nothing_an_include_list_names() {
        let rule = rule_with(serde_json::json!(["check_run"]), false, false);
        let config = GithubPrRuleConfig::from_rule(&rule);
        assert!(config.mutes(&FilterKey::new("issue_comment", "")));
        assert!(!config.mutes(&FilterKey::default()));
    }

    /// An entry the parser skips is reported rather than read as a working
    /// filter; a scoped `{login, events}` sender is not one of them.
    #[test]
    fn unparsed_filter_entries_are_reported_as_inert() {
        let config = serde_json::json!({
            "ignored_senders": [
                "linear-code[bot]",
                {"login": "github-actions[bot]", "events": ["issue_comment"]},
                {"events": ["issue_comment"]},
            ],
            "ignored_events": ["check_run", 7],
        });
        assert_eq!(
            inert_filter_entries(Some(&config)),
            [
                &serde_json::json!(7),
                &serde_json::json!({"events": ["issue_comment"]})
            ]
        );
        assert!(inert_filter_entries(None).is_empty());
    }
}
