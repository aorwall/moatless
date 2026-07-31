//! External status capability and display fields for GitHub PR bindings.

use tracing::warn;

use crate::adapters::AdapterBindingStatus;
use crate::adapters::models::{ExternalStatus, TaskBinding};
use crate::bindings::dao::PgTaskBindingDao;
use crate::db::DbPool;

use super::adapter::GithubPrAdapter;
use super::api::PrInfo;
use super::signals::ReviewDecision;
use super::subject::GithubPrSubject;

/// One idempotent status projection derived during webhook normalization.
pub(super) enum WebhookStatusObservation {
    PullRequest {
        subject_key: String,
        status: ExternalStatus,
    },
    ReviewDecision {
        subject_key: String,
        decision: ReviewDecision,
    },
}

/// Apply a normalized webhook observation best-effort and off the response path.
pub(super) fn apply_webhook_observation(db: DbPool, observation: WebhookStatusObservation) {
    tokio::spawn(async move {
        let repository = PgTaskBindingDao(db);
        let subject_key = match &observation {
            WebhookStatusObservation::PullRequest { subject_key, .. }
            | WebhookStatusObservation::ReviewDecision { subject_key, .. } => subject_key,
        };
        let bindings = match repository
            .find_by_subject("github", "github_pr", subject_key)
            .await
        {
            Ok(bindings) => bindings,
            Err(error) => {
                warn!(
                    %subject_key,
                    %error,
                    "Failed to look up bindings for GitHub webhook status persistence"
                );
                return;
            }
        };

        for binding in bindings {
            let result = match &observation {
                WebhookStatusObservation::PullRequest { status, .. } => {
                    repository
                        .update_external_status(
                            &binding.id,
                            status.status.as_deref(),
                            status.state_kind.as_deref(),
                            &status.raw,
                        )
                        .await
                }
                WebhookStatusObservation::ReviewDecision { decision, .. } => {
                    repository
                        .update_review_decision(&binding.id, Some(decision.as_str()))
                        .await
                }
            };
            if let Err(error) = result {
                warn!(
                    binding_id = %binding.id,
                    %error,
                    "Failed to persist GitHub webhook status observation"
                );
            }
        }
    });
}

/// Normalized state kinds a PR can no longer move away from on its own. A
/// reopened PR leaves this set, but only a webhook can tell us so.
const TERMINAL_STATE_KINDS: [&str; 2] = ["merged", "closed"];

/// Derive the normalized status of a GitHub PR from its raw fields.
///
/// GitHub reports both merged and abandoned PRs as `state = "closed"`, so the
/// `merged` flag is needed to distinguish them. Draft is only meaningful while
/// the PR is open.
pub(super) fn github_external_status(
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
fn status_from_pr_info(pr: &PrInfo) -> ExternalStatus {
    github_external_status(
        &pr.state,
        pr.merged.unwrap_or(false),
        pr.draft.unwrap_or(false),
        Some(pr.title.as_str()),
        pr.html_url.as_deref(),
    )
}

/// Stable key naming the pull request a binding points at, so several tasks
/// bound to the same PR share one cache entry and one API call. `None` when the
/// binding does not carry a resolvable PR.
fn binding_status_key(binding: &TaskBinding) -> Option<String> {
    GithubPrSubject::from_destination_config(&binding.destination_config)
        .map(|subject| subject.status_key())
}

/// Whether a normalized state kind means the PR is finished.
fn is_terminal_state_kind(state_kind: &str) -> bool {
    TERMINAL_STATE_KINDS.contains(&state_kind)
}

/// Human label and web URL for the pull request a binding points at.
///
/// The label is the binding's `subject_key`, which is already `owner/repo#N`.
/// The URL is built from the destination config, falling back to the `url` a
/// previously persisted status recorded — bindings created before the config
/// carried the PR coordinates still render a link.
fn binding_label_and_url(binding: &TaskBinding) -> (String, Option<String>) {
    let url = pull_request_url(binding).or_else(|| status_raw_url(binding));
    (binding.subject_key.clone(), url)
}

fn pull_request_url(binding: &TaskBinding) -> Option<String> {
    GithubPrSubject::from_destination_config(&binding.destination_config)
        .map(|subject| subject.pull_request_url())
}

fn status_raw_url(binding: &TaskBinding) -> Option<String> {
    binding
        .external_status_raw
        .as_ref()?
        .get("url")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[async_trait::async_trait]
impl AdapterBindingStatus for GithubPrAdapter {
    fn binding_status_key(&self, binding: &TaskBinding) -> Option<String> {
        binding_status_key(binding)
    }

    fn binding_label_and_url(&self, binding: &TaskBinding) -> (String, Option<String>) {
        binding_label_and_url(binding)
    }

    fn is_terminal_state_kind(&self, state_kind: &str) -> bool {
        is_terminal_state_kind(state_kind)
    }

    async fn fetch_binding_status(
        &self,
        binding: &TaskBinding,
        acting_user: &str,
    ) -> Option<ExternalStatus> {
        let subject = GithubPrSubject::from_destination_config(&binding.destination_config)?;

        let api = match self
            .auth()
            .client_for_live_status(acting_user, subject.owner())
            .await
        {
            Ok(api) => api,
            Err(error) => {
                warn!(
                    binding_id = %binding.id,
                    error = %error,
                    "Failed to resolve GitHub token for live PR status"
                );
                return None;
            }
        };

        match api
            .get_pr(subject.owner(), subject.repo(), subject.pr_number())
            .await
        {
            Ok(pr) => Some(status_from_pr_info(&pr)),
            Err(error) => {
                warn!(
                    binding_id = %binding.id,
                    error = %error,
                    "Failed to fetch live GitHub PR status"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::{Method, StatusCode};

    use super::super::test_support::{StubResponse, start_github_api_stub};
    use super::*;
    use crate::auth::github::registry::GitHubAppRegistry;
    use crate::auth::test_support::{StubProviderTokens, github_row};

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

    #[tokio::test]
    async fn live_status_refresh_keeps_exact_github_request_budget() {
        let stub = start_github_api_stub(|method, path, uri| match (method, path) {
            (Method::GET, "/repos/soaplabs/repo/pulls/42") => StubResponse::json(
                StatusCode::OK,
                serde_json::json!({
                    "number": 42,
                    "title": "Characterize contracts",
                    "body": "Body",
                    "state": "open",
                    "merged": false,
                    "mergeable": true,
                    "mergeable_state": "clean",
                    "draft": false,
                    "html_url": "https://github.com/soaplabs/repo/pull/42",
                    "user": {"login": "alice"},
                    "head": {"ref": "feature", "sha": "abc123"},
                    "base": {"ref": "main", "sha": "def456"}
                }),
            ),
            _ => StubResponse::not_found(uri),
        })
        .await;

        let provider = GithubPrAdapter::new(
            None,
            Some("wrong-global-token".to_string()),
            "https://app.example.com".to_string(),
            Arc::new(StubProviderTokens::with_user_row_for(
                "owner-user",
                github_row("manual", "owner-token"),
            )),
            None,
            None,
            Arc::new(GitHubAppRegistry::default()),
        )
        .with_api_base(stub.base_url());
        let status = provider
            .fetch_binding_status(
                &binding_with_config(serde_json::json!({
                    "owner": "soaplabs",
                    "repo": "repo",
                    "pr_number": 42,
                })),
                "owner-user",
            )
            .await
            .expect("one live PR read returns status");
        assert_eq!(status.state_kind.as_deref(), Some("open"));

        // Every request below was provoked by this test: the stub is reached
        // only by the client this Adapter built against it.
        let recorded = stub.recorded_requests().await;
        let selected_status_token = Some("Bearer owner-token");
        let status_requests: Vec<_> = recorded.iter().collect();
        let observed: Vec<(&str, &str)> = status_requests
            .iter()
            .map(|request| (request.method.as_str(), request.uri.as_str()))
            .collect();
        assert_eq!(observed, vec![("GET", "/repos/soaplabs/repo/pulls/42")]);
        assert!(
            status_requests
                .iter()
                .all(|request| request.authorization.as_deref() == selected_status_token),
            "status refresh keeps its selected user token: {recorded:?}"
        );
    }
}
