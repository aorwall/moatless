//! Connection identity, status projection, and message-count attribution.

use moatless_adapter_model::{capabilities::AdapterBindingStatus, registry::Adapters};
use moatless_api_schemas::task_connections::Connection;
use moatless_tasks::message::dao::ConnectionMessageCount;
use moatless_types::adapter::TaskBinding;
use std::collections::HashMap;
use std::sync::Arc;

/// Normalize an adapter/provider string to a canonical bucket so that the
/// GitHub binding provider (`github`) and the GitHub message adapter label
/// (`github_pr`) compare equal.
///
/// Deliberately a total, registry-independent fold: both sides of the join in
/// [`match_counts`] go through it and the bucket never leaves this module, so
/// resolving it through the adapter registry would only add a way for the join
/// to miss (and message counts to read zero) when that adapter is unconfigured.
fn normalize_adapter(s: &str) -> &str {
    match s {
        "github" | "github_pr" => "github",
        other => other,
    }
}

pub(super) fn group_counts_by_task(
    counts: &[ConnectionMessageCount],
) -> HashMap<&str, Vec<&ConnectionMessageCount>> {
    let mut map: HashMap<&str, Vec<&ConnectionMessageCount>> = HashMap::new();
    for c in counts {
        map.entry(c.task_id.as_str()).or_default().push(c);
    }
    map
}

pub(super) fn group_bindings_by_task(bindings: &[TaskBinding]) -> HashMap<&str, Vec<&TaskBinding>> {
    let mut map: HashMap<&str, Vec<&TaskBinding>> = HashMap::new();
    for b in bindings {
        map.entry(b.task_id.as_str()).or_default().push(b);
    }
    map
}

/// Attribute message counts to bindings. Exact `subject_key` match wins;
/// counts whose `subject_key` matches no binding fall back to a per-adapter
/// bucket, applied only when that adapter has a single binding on the task.
pub(super) fn match_counts(
    task_bindings: &[&TaskBinding],
    rows: &[&ConnectionMessageCount],
) -> HashMap<String, (i64, i64)> {
    let mut out: HashMap<String, (i64, i64)> = HashMap::new();

    let mut subject_to_binding: HashMap<&str, &TaskBinding> = HashMap::new();
    for b in task_bindings {
        subject_to_binding
            .entry(b.subject_key.as_str())
            .or_insert(b);
    }

    let mut bindings_by_adapter: HashMap<&str, Vec<&TaskBinding>> = HashMap::new();
    for b in task_bindings {
        bindings_by_adapter
            .entry(normalize_adapter(b.adapter_kind.as_str()))
            .or_default()
            .push(b);
    }

    let mut leftover: HashMap<String, (i64, i64)> = HashMap::new();
    for row in rows {
        if let Some(sk) = row.external_subject_key.as_deref() {
            if let Some(b) = subject_to_binding.get(sk) {
                let entry = out.entry(b.id.clone()).or_insert((0, 0));
                entry.0 += row.incoming;
                entry.1 += row.outgoing;
                continue;
            }
        }
        let adapter = row
            .external_adapter
            .as_deref()
            .map_or("", normalize_adapter)
            .to_string();
        let entry = leftover.entry(adapter).or_insert((0, 0));
        entry.0 += row.incoming;
        entry.1 += row.outgoing;
    }

    for (adapter, (inc, outg)) in leftover {
        if let Some(bs) = bindings_by_adapter.get(adapter.as_str()) {
            if bs.len() == 1 {
                let entry = out.entry(bs[0].id.clone()).or_insert((0, 0));
                entry.0 += inc;
                entry.1 += outg;
            }
        }
    }

    out
}

/// The adapter that owns a binding's provider, if one is configured.
pub(super) fn binding_status_for<'a>(
    adapters: Option<&'a Arc<Adapters>>,
    provider: &str,
) -> Option<&'a dyn AdapterBindingStatus> {
    adapters?.binding_status_by_name(provider)
}

/// Decode a binding's `destination_config` + persisted status into a typed
/// [`Connection`].
///
/// The label and URL come from the binding's status capability, which is the only thing
/// that knows the shape of its own destination config. `adapter` is the one
/// resolved for this binding's provider; `None` (unconfigured or unknown
/// provider) falls back to the same default the trait gives.
pub(super) fn decode_connection(
    status: Option<&dyn AdapterBindingStatus>,
    b: &TaskBinding,
    counts: &HashMap<String, (i64, i64)>,
) -> Connection {
    let (label, url) = status.map_or_else(
        || (b.subject_key.clone(), None),
        |status| status.binding_label_and_url(b),
    );
    let (incoming, outgoing) = counts.get(&b.id).copied().unwrap_or((0, 0));

    Connection {
        binding_id: b.id.clone(),
        adapter_kind: b.adapter_kind,
        kind: connection_kind(&b.subject_kind),
        label,
        url,
        status: b.external_status.clone(),
        state_kind: b.external_state_kind.clone(),
        status_updated_at: b.external_status_updated_at.map(|t| t.to_rfc3339()),
        review_decision: b.external_review_decision.clone(),
        mergeable_state: b.external_mergeable_state.clone(),
        incoming,
        outgoing,
        last_event_at: Some(b.last_event_at.to_rfc3339()),
        active: b.active,
        deactivated_at: b.deactivated_at.map(|t| t.to_rfc3339()),
    }
}

fn connection_kind(subject_kind: &str) -> String {
    match subject_kind {
        "github_pr" => "github_pr".to_string(),
        "linear_issue" | "linear_agent_session" => "linear_issue".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use moatless_adapter_model::capabilities::AdapterBindingStatus;

    use super::*;

    /// Stand-in adapter: names its own bindings, which is all this module asks
    /// of an adapter when it decodes a connection.
    struct StubAdapter;

    #[async_trait::async_trait]
    impl AdapterBindingStatus for StubAdapter {
        fn binding_label_and_url(&self, binding: &TaskBinding) -> (String, Option<String>) {
            (
                format!("stub {}", binding.subject_key),
                Some("https://stub.example/1".to_string()),
            )
        }
    }

    fn binding(id: &str, provider: &str, subject_kind: &str, subject_key: &str) -> TaskBinding {
        let now = Utc::now();
        TaskBinding {
            id: id.to_string(),
            task_id: "task_1".to_string(),
            adapter_kind: moatless_types::adapter::AdapterKind::parse_or_webhook(provider),
            subject_kind: subject_kind.to_string(),
            subject_key: subject_key.to_string(),
            connection_id: "conn_1".to_string(),
            loop_id: None,
            destination_config: serde_json::json!({}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: "ingress".to_string(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
        }
    }

    fn count(
        subject_key: Option<&str>,
        adapter: Option<&str>,
        incoming: i64,
        outgoing: i64,
    ) -> ConnectionMessageCount {
        ConnectionMessageCount {
            task_id: "task_1".to_string(),
            external_adapter: adapter.map(str::to_string),
            external_subject_key: subject_key.map(str::to_string),
            incoming,
            outgoing,
        }
    }

    #[test]
    fn linear_kind_normalizes_both_subject_kinds() {
        assert_eq!(connection_kind("linear_agent_session"), "linear_issue");
        assert_eq!(connection_kind("linear_issue"), "linear_issue");
        assert_eq!(connection_kind("github_pr"), "github_pr");
        assert_eq!(connection_kind("slack_thread"), "slack_thread");
    }

    #[test]
    fn counts_match_on_subject_key() {
        let b1 = binding("b1", "github", "github_pr", "o/r#1");
        let b2 = binding("b2", "github", "github_pr", "o/r#2");
        let bindings = vec![&b1, &b2];
        let c1 = count(Some("o/r#1"), Some("github_pr"), 3, 1);
        let c2 = count(Some("o/r#2"), Some("github_pr"), 5, 2);
        let rows = vec![&c1, &c2];

        let matched = match_counts(&bindings, &rows);
        assert_eq!(matched.get("b1").copied(), Some((3, 1)));
        assert_eq!(matched.get("b2").copied(), Some((5, 2)));
    }

    #[test]
    fn counts_without_subject_fall_back_to_sole_adapter_binding() {
        // Single linear binding; a legacy row with null subject_key should be
        // attributed to it.
        let b1 = binding("b1", "linear", "linear_agent_session", "session-uuid");
        let bindings = vec![&b1];
        let legacy = count(None, Some("linear"), 4, 6);
        let rows = vec![&legacy];

        let matched = match_counts(&bindings, &rows);
        assert_eq!(matched.get("b1").copied(), Some((4, 6)));
    }

    #[test]
    fn two_provider_spellings_fold_together() {
        // The binding is stored under the connection key, the message row under
        // the message key; `normalize_adapter` folds them so the join lands.
        let b1 = binding("b1", "github", "github_pr", "o/r#1");
        let bindings = vec![&b1];
        let legacy = count(None, Some("github_pr"), 2, 3);
        let rows = vec![&legacy];

        let matched = match_counts(&bindings, &rows);
        assert_eq!(matched.get("b1").copied(), Some((2, 3)));
    }

    #[test]
    fn ambiguous_adapter_fallback_is_dropped() {
        // Two bindings of the same adapter — a null-subject row can't be
        // attributed unambiguously, so it's dropped rather than double-counted.
        let b1 = binding("b1", "github", "github_pr", "o/r#1");
        let b2 = binding("b2", "github", "github_pr", "o/r#2");
        let bindings = vec![&b1, &b2];
        let orphan = count(None, Some("github_pr"), 9, 9);
        let rows = vec![&orphan];

        let matched = match_counts(&bindings, &rows);
        assert!(matched.is_empty());
    }

    #[test]
    fn decode_connection_takes_label_and_url_from_the_adapter() {
        let counts: HashMap<String, (i64, i64)> = HashMap::new();
        let b = binding("b1", "webhook", "stub_subject", "o/r#1");

        let decoded = decode_connection(Some(&StubAdapter), &b, &counts);
        assert_eq!(decoded.label, "stub o/r#1");
        assert_eq!(decoded.url.as_deref(), Some("https://stub.example/1"));
    }

    #[test]
    fn decode_connection_falls_back_when_no_adapter_answers() {
        let counts: HashMap<String, (i64, i64)> = HashMap::new();
        let b = binding("b1", "not_configured", "stub_subject", "o/r#1");

        let decoded = decode_connection(None, &b, &counts);
        assert_eq!(decoded.label, "o/r#1");
        assert!(decoded.url.is_none());
    }

    #[test]
    fn decode_connection_carries_active_state() {
        let counts: HashMap<String, (i64, i64)> = HashMap::new();

        let active = binding("b1", "github", "github_pr", "o/r#1");
        let decoded = decode_connection(Some(&StubAdapter), &active, &counts);
        assert!(decoded.active);
        assert_eq!(decoded.deactivated_at, None);

        let mut closed = binding("b2", "github", "github_pr", "o/r#2");
        closed.active = false;
        closed.deactivated_at = Some(Utc::now());
        let decoded = decode_connection(Some(&StubAdapter), &closed, &counts);
        assert!(!decoded.active);
        assert!(decoded.deactivated_at.is_some());
    }
}
