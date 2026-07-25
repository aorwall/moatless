//! External status and display fields for Linear issue bindings.
//!
//! Free functions so they can be unit-tested without constructing a
//! [`LinearAdapter`](super::adapter::LinearAdapter) (which needs a database
//! pool). The `Adapter` trait methods delegate here.

use crate::adapters::models::{ExternalStatus, TaskBinding};

/// Linear `state.type` values that mean the issue is finished. A reactivated
/// issue leaves this set, but only a webhook can tell us so.
const TERMINAL_STATE_KINDS: [&str; 2] = ["completed", "canceled"];

/// Derive the normalized status of a Linear issue from its workflow state.
///
/// The display string is the (user-editable) state name; the normalized bucket
/// is Linear's stable `state.type`.
pub fn linear_external_status(
    state_name: &str,
    state_type: &str,
    title: Option<&str>,
    url: Option<&str>,
) -> ExternalStatus {
    ExternalStatus {
        status: (!state_name.is_empty()).then(|| state_name.to_string()),
        state_kind: (!state_type.is_empty()).then(|| state_type.to_string()),
        raw: serde_json::json!({
            "stateName": state_name,
            "stateType": state_type,
            "title": title,
            "url": url,
        }),
    }
}

/// Derive an [`ExternalStatus`] from an issue payload returned by the Linear
/// API (`state.name` / `state.type`, plus title and url).
pub(crate) fn status_from_issue(issue: &serde_json::Value) -> ExternalStatus {
    let state_name = issue
        .pointer("/state/name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let state_type = issue
        .pointer("/state/type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let title = issue.get("title").and_then(serde_json::Value::as_str);
    let url = issue.get("url").and_then(serde_json::Value::as_str);
    linear_external_status(state_name, state_type, title, url)
}

/// The issue a binding points at, plus the workspace whose token can read it.
pub(crate) fn binding_subject(binding: &TaskBinding) -> Option<(&str, &str)> {
    let issue_id = binding
        .destination_config
        .get("issue_id")
        .and_then(serde_json::Value::as_str)?;
    let org_id = binding
        .destination_config
        .get("team_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    Some((issue_id, org_id))
}

/// Stable key naming the issue a binding points at, so several tasks bound to
/// the same issue share one cache entry and one API call. `None` when the
/// binding does not carry a resolvable issue.
pub(crate) fn binding_status_key(binding: &TaskBinding) -> Option<String> {
    let (issue_id, _) = binding_subject(binding)?;
    Some(format!("lin:{issue_id}"))
}

/// Whether a Linear `state.type` means the issue is finished.
pub(crate) fn is_terminal_state_kind(state_kind: &str) -> bool {
    TERMINAL_STATE_KINDS.contains(&state_kind)
}

/// Human label and web URL for the issue a binding points at.
///
/// The label prefers the issue identifier (`SP-1721`) the destination config
/// records under either of the two keys it has used, falling back to the
/// binding's `subject_key` — which for agent-session bindings is an opaque
/// UUID, hence the preference.
pub(crate) fn binding_label_and_url(binding: &TaskBinding) -> (String, Option<String>) {
    let cfg = &binding.destination_config;
    let label = config_str(cfg, "identifier")
        .or_else(|| config_str(cfg, "issue_identifier"))
        .unwrap_or_else(|| binding.subject_key.clone());
    let url = config_str(cfg, "issue_url").or_else(|| config_str(cfg, "url"));
    (label, url)
}

/// Read a non-empty string field out of a destination config.
fn config_str(cfg: &serde_json::Value, key: &str) -> Option<String> {
    cfg.get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding_with_config(config: serde_json::Value) -> TaskBinding {
        let now = chrono::Utc::now();
        TaskBinding {
            id: "b1".into(),
            task_id: "t1".into(),
            provider: "linear".into(),
            subject_kind: "linear_issue".into(),
            subject_key: "issue-uuid".into(),
            connection_id: "c1".into(),
            trigger_id: None,
            destination_config: config,
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: "ingress".into(),
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

    #[test]
    fn linear_uses_name_and_type() {
        let s = linear_external_status("In Review", "started", Some("t"), Some("u"));
        assert_eq!(s.status.as_deref(), Some("In Review"));
        assert_eq!(s.state_kind.as_deref(), Some("started"));
    }

    #[test]
    fn linear_empty_fields_are_none() {
        let s = linear_external_status("", "", None, None);
        assert!(s.status.is_none());
        assert!(s.state_kind.is_none());
    }

    #[test]
    fn status_from_issue_reads_nested_state() {
        let issue = serde_json::json!({
            "title": "Fix login",
            "url": "https://linear.app/acme/issue/SP-1",
            "state": { "name": "Done", "type": "completed" },
        });
        let s = status_from_issue(&issue);
        assert_eq!(s.status.as_deref(), Some("Done"));
        assert_eq!(s.state_kind.as_deref(), Some("completed"));
    }

    #[test]
    fn status_key_names_the_issue() {
        let binding = binding_with_config(serde_json::json!({ "issue_id": "abc-123" }));
        assert_eq!(binding_status_key(&binding).as_deref(), Some("lin:abc-123"));
    }

    #[test]
    fn status_key_is_none_without_an_issue() {
        let binding = binding_with_config(serde_json::json!({ "session_id": "s1" }));
        assert!(binding_status_key(&binding).is_none());
    }

    #[test]
    fn label_prefers_identifier() {
        let binding = binding_with_config(serde_json::json!({
            "identifier": "SP-1721",
            "issue_url": "https://linear.app/x/issue/SP-1721",
        }));
        let (label, url) = binding_label_and_url(&binding);
        assert_eq!(label, "SP-1721");
        assert_eq!(url.as_deref(), Some("https://linear.app/x/issue/SP-1721"));
    }

    #[test]
    fn label_falls_back_through_both_identifier_keys_then_subject_key() {
        let legacy = binding_with_config(serde_json::json!({ "issue_identifier": "SP-9" }));
        assert_eq!(binding_label_and_url(&legacy).0, "SP-9");

        let bare = binding_with_config(serde_json::json!({}));
        assert_eq!(binding_label_and_url(&bare).0, "issue-uuid");

        // Empty strings are not labels.
        let empty = binding_with_config(serde_json::json!({ "identifier": "" }));
        assert_eq!(binding_label_and_url(&empty).0, "issue-uuid");
    }

    #[test]
    fn url_falls_back_from_issue_url_to_url() {
        let binding = binding_with_config(serde_json::json!({
            "url": "https://linear.app/x/issue/SP-2",
        }));
        assert_eq!(
            binding_label_and_url(&binding).1.as_deref(),
            Some("https://linear.app/x/issue/SP-2")
        );

        let none = binding_with_config(serde_json::json!({}));
        assert!(binding_label_and_url(&none).1.is_none());
    }

    #[test]
    fn terminal_kinds_are_completed_and_canceled() {
        assert!(is_terminal_state_kind("completed"));
        assert!(is_terminal_state_kind("canceled"));
        assert!(!is_terminal_state_kind("started"));
        assert!(!is_terminal_state_kind("backlog"));
    }
}
