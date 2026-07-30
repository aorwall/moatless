//! External status and display fields for GitHub PR bindings.
//!
//! Free functions so they can be unit-tested without constructing a
//! [`GithubPrAdapter`](super::adapter::GithubPrAdapter) (which needs a database
//! pool and a token repository). The `Adapter` trait methods delegate here.

use crate::adapters::models::{ExternalStatus, TaskBinding};

use super::api::PrInfo;

/// Normalized state kinds a PR can no longer move away from on its own. A
/// reopened PR leaves this set, but only a webhook can tell us so.
const TERMINAL_STATE_KINDS: [&str; 2] = ["merged", "closed"];

/// Derive the normalized status of a GitHub PR from its raw fields.
///
/// GitHub reports both merged and abandoned PRs as `state = "closed"`, so the
/// `merged` flag is needed to distinguish them. Draft is only meaningful while
/// the PR is open.
pub fn github_external_status(
    state: &str,
    merged: bool,
    draft: bool,
    title: Option<&str>,
    url: Option<&str>,
) -> ExternalStatus {
    let kind = if merged {
        "merged"
    } else if state.eq_ignore_ascii_case("closed") {
        "closed"
    } else if draft {
        "draft"
    } else {
        "open"
    };

    ExternalStatus {
        status: Some(kind.to_string()),
        state_kind: Some(kind.to_string()),
        raw: serde_json::json!({
            "title": title,
            "url": url,
            "state": state,
            "merged": merged,
            "draft": draft,
        }),
    }
}

/// Derive an [`ExternalStatus`] directly from a fetched [`PrInfo`].
pub(crate) fn status_from_pr_info(pr: &PrInfo) -> ExternalStatus {
    github_external_status(
        &pr.state,
        pr.merged.unwrap_or(false),
        pr.draft.unwrap_or(false),
        Some(pr.title.as_str()),
        pr.html_url.as_deref(),
    )
}

/// Parse `(owner, repo, pr_number)` from a GitHub binding's destination config.
pub(crate) fn binding_subject(binding: &TaskBinding) -> Option<(String, String, u64)> {
    let cfg = &binding.destination_config;
    let owner = cfg.get("owner").and_then(serde_json::Value::as_str)?;
    let repo = cfg.get("repo").and_then(serde_json::Value::as_str)?;
    let number = cfg.get("pr_number").and_then(serde_json::Value::as_u64)?;
    Some((owner.to_string(), repo.to_string(), number))
}

/// Stable key naming the pull request a binding points at, so several tasks
/// bound to the same PR share one cache entry and one API call. `None` when the
/// binding does not carry a resolvable PR.
pub(crate) fn binding_status_key(binding: &TaskBinding) -> Option<String> {
    let (owner, repo, number) = binding_subject(binding)?;
    Some(format!("gh:{owner}/{repo}#{number}"))
}

/// Whether a normalized state kind means the PR is finished.
pub(crate) fn is_terminal_state_kind(state_kind: &str) -> bool {
    TERMINAL_STATE_KINDS.contains(&state_kind)
}

/// Human label and web URL for the pull request a binding points at.
///
/// The label is the binding's `subject_key`, which is already `owner/repo#N`.
/// The URL is built from the destination config, falling back to the `url` a
/// previously persisted status recorded — bindings created before the config
/// carried the PR coordinates still render a link.
pub(crate) fn binding_label_and_url(binding: &TaskBinding) -> (String, Option<String>) {
    let url = pull_request_url(binding).or_else(|| status_raw_url(binding));
    (binding.subject_key.clone(), url)
}

fn pull_request_url(binding: &TaskBinding) -> Option<String> {
    let (owner, repo, number) = binding_subject(binding)?;
    Some(format!("https://github.com/{owner}/{repo}/pull/{number}"))
}

fn status_raw_url(binding: &TaskBinding) -> Option<String> {
    binding
        .external_status_raw
        .as_ref()?
        .get("url")
        .and_then(serde_json::Value::as_str)
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
            provider: "github".into(),
            subject_kind: "github_pr".into(),
            subject_key: "acme/web#7".into(),
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
    fn github_merged_beats_closed_and_draft() {
        let s = github_external_status("closed", true, false, Some("t"), Some("u"));
        assert_eq!(s.status.as_deref(), Some("merged"));
        assert_eq!(s.state_kind.as_deref(), Some("merged"));
    }

    #[test]
    fn github_closed_not_merged() {
        let s = github_external_status("closed", false, false, None, None);
        assert_eq!(s.status.as_deref(), Some("closed"));
    }

    #[test]
    fn github_draft_when_open() {
        let s = github_external_status("open", false, true, None, None);
        assert_eq!(s.status.as_deref(), Some("draft"));
        assert_eq!(s.state_kind.as_deref(), Some("draft"));
    }

    #[test]
    fn github_plain_open() {
        let s = github_external_status("open", false, false, None, None);
        assert_eq!(s.status.as_deref(), Some("open"));
    }

    #[test]
    fn status_key_names_the_pull_request() {
        let binding = binding_with_config(serde_json::json!({
            "owner": "acme",
            "repo": "web",
            "pr_number": 7,
        }));
        assert_eq!(
            binding_status_key(&binding).as_deref(),
            Some("gh:acme/web#7")
        );
    }

    #[test]
    fn status_key_is_none_without_a_pull_request() {
        let binding = binding_with_config(serde_json::json!({ "owner": "acme", "repo": "web" }));
        assert!(binding_status_key(&binding).is_none());
    }

    #[test]
    fn label_is_subject_key_and_url_is_derived() {
        let binding = binding_with_config(serde_json::json!({
            "owner": "o",
            "repo": "r",
            "pr_number": 42,
        }));
        let (label, url) = binding_label_and_url(&binding);
        assert_eq!(label, "acme/web#7");
        assert_eq!(url.as_deref(), Some("https://github.com/o/r/pull/42"));
    }

    #[test]
    fn url_falls_back_to_the_persisted_status() {
        let mut binding = binding_with_config(serde_json::json!({}));
        binding.external_status_raw = Some(serde_json::json!({
            "url": "https://github.com/o/r/pull/9",
        }));
        let (_, url) = binding_label_and_url(&binding);
        assert_eq!(url.as_deref(), Some("https://github.com/o/r/pull/9"));
    }

    #[test]
    fn url_is_none_without_config_or_persisted_status() {
        let binding = binding_with_config(serde_json::json!({}));
        let (label, url) = binding_label_and_url(&binding);
        assert_eq!(label, "acme/web#7");
        assert!(url.is_none());
    }

    #[test]
    fn terminal_kinds_are_merged_and_closed() {
        assert!(is_terminal_state_kind("merged"));
        assert!(is_terminal_state_kind("closed"));
        assert!(!is_terminal_state_kind("open"));
        assert!(!is_terminal_state_kind("draft"));
    }
}
