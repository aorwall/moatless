//! External status capability and display fields for GitHub PR bindings.

use moatless_workspace_types::repository::GitHostRepo;
use tracing::warn;

use moatless_adapter_model::capabilities::AdapterBindingStatus;
use moatless_types::adapter::ExternalStatus;
use moatless_types::adapter::TaskBinding;
use std::sync::Arc;

use moatless_loops::bindings::operations::Bindings;

use super::adapter::GithubPrAdapter;
use super::pull_request_conversation::pull_request_data;
use super::pull_request_search::Nodes;
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

/// Apply a normalized webhook observation best-effort and off the response
/// path.
pub(super) fn apply_webhook_observation(
    repository: Arc<Bindings>,
    observation: WebhookStatusObservation,
) {
    tokio::spawn(async move {
        let subject_key = match &observation {
            WebhookStatusObservation::PullRequest { subject_key, .. }
            | WebhookStatusObservation::ReviewDecision { subject_key, .. } => subject_key,
        };
        let bindings = match repository
            .for_subject("github", "github_pr", subject_key)
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
                        .merge_external_status(
                            &binding.id,
                            status.status.as_deref(),
                            status.state_kind.as_deref(),
                            &status.raw,
                        )
                        .await
                }
                WebhookStatusObservation::ReviewDecision { decision, .. } => {
                    repository
                        .record_review_decision(&binding.id, Some(decision.as_str()))
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

/// A GitHub PR's raw fields, as a webhook payload or a live read carries them.
#[derive(Default)]
pub(super) struct PullRequestFields<'a> {
    /// `open` or `closed`; GitHub reports a merged PR as `closed`.
    pub(super) state: &'a str,
    pub(super) merged: bool,
    pub(super) draft: bool,
    pub(super) title: Option<&'a str>,
    pub(super) url: Option<&'a str>,
    pub(super) base_ref: Option<&'a str>,
    pub(super) head_ref: Option<&'a str>,
    /// GitHub's own last change to the PR, ISO 8601.
    pub(super) updated_at: Option<&'a str>,
}

/// Derive the normalized status of a GitHub PR from its raw fields.
///
/// GitHub reports both merged and abandoned PRs as `state = "closed"`, so
/// `merged` is needed to distinguish them; `draft` is only meaningful while the
/// PR is open. The blob carries no `checksState`: [`status_from_live_read`]
/// adds the rollup, and `pr_status_observation` clears it on a push.
pub(super) fn github_external_status(pr: &PullRequestFields<'_>) -> ExternalStatus {
    let kind = if pr.merged {
        "merged"
    } else if pr.state.eq_ignore_ascii_case("closed") {
        "closed"
    } else if pr.draft {
        "draft"
    } else {
        "open"
    };

    ExternalStatus {
        status: Some(kind.to_string()),
        state_kind: Some(kind.to_string()),
        raw: serde_json::json!({
            "title": pr.title,
            "url": pr.url,
            "state": pr.state,
            "merged": pr.merged,
            "draft": pr.draft,
            "baseRef": pr.base_ref,
            "headRef": pr.head_ref,
            "updatedAt": pr.updated_at,
        }),
    }
}

/// A head commit's `statusCheckRollup.state` as T3's `PullRequestChecksState`,
/// or `None` for a state it has no word for.
pub(super) fn checks_state(rollup_state: &str) -> Option<&'static str> {
    match rollup_state {
        "SUCCESS" => Some("passing"),
        "FAILURE" | "ERROR" => Some("failing"),
        "PENDING" | "EXPECTED" => Some("pending"),
        _ => None,
    }
}

/// One read of everything the status blob holds, the checks rollup included.
const LIVE_STATUS_QUERY: &str = r#"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      title url state isDraft baseRefName headRefName updatedAt
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
    }
  }
}"#;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LivePullRequest {
    title: String,
    url: Option<String>,
    /// `OPEN`, `CLOSED` or `MERGED`.
    state: String,
    #[serde(default)]
    is_draft: bool,
    base_ref_name: String,
    head_ref_name: String,
    updated_at: Option<String>,
    #[serde(default)]
    commits: Nodes<LiveCommitNode>,
}

#[derive(serde::Deserialize)]
struct LiveCommitNode {
    commit: LiveCommit,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveCommit {
    status_check_rollup: Option<LiveRollup>,
}

#[derive(serde::Deserialize)]
struct LiveRollup {
    state: String,
}

/// Derive an [`ExternalStatus`] from a [`LIVE_STATUS_QUERY`] read.
///
/// `checksState` is `null` for a head commit with no checks, which T3 reads as
/// no tone rather than pending.
fn status_from_live_read(pr: &LivePullRequest) -> ExternalStatus {
    let merged = pr.state == "MERGED";
    let state = if pr.state == "OPEN" { "open" } else { "closed" };
    let mut status = github_external_status(&PullRequestFields {
        state,
        merged,
        draft: pr.is_draft,
        title: Some(pr.title.as_str()),
        url: pr.url.as_deref(),
        base_ref: Some(pr.base_ref_name.as_str()),
        head_ref: Some(pr.head_ref_name.as_str()),
        updated_at: pr.updated_at.as_deref(),
    });
    let checks = pr
        .commits
        .nodes
        .iter()
        .flatten()
        .next_back()
        .and_then(|node| node.commit.status_check_rollup.as_ref())
        .and_then(|rollup| checks_state(&rollup.state));
    status.raw["checksState"] = serde_json::json!(checks);
    status
}

/// The base and head branch names a persisted GitHub status blob recorded, if
/// it was written since this adapter began carrying them. `None` for a blob
/// that predates the change, which is what marks it stale for one refresh.
pub(super) fn status_base_head(binding: &TaskBinding) -> Option<(String, String)> {
    let raw = binding.external_status_raw.as_ref()?;
    let base = raw.get("baseRef").and_then(serde_json::Value::as_str)?;
    let head = raw.get("headRef").and_then(serde_json::Value::as_str)?;
    Some((base.to_string(), head.to_string()))
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
/// The label is the binding's `subject_key` (already `owner/repo#N`); the URL
/// falls back to a previously persisted status's `url` so bindings created
/// before the config carried PR coordinates still render a link.
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

    fn persisted_status_is_complete(&self, binding: &TaskBinding) -> bool {
        // A blob written before base and head were carried holds neither, so it
        // is refreshed once to gain them — even a terminal pull request, whose
        // one refresh reads the branches its status never recorded.
        binding.external_status_raw.is_none() || status_base_head(binding).is_some()
    }

    async fn fetch_binding_status(
        &self,
        binding: &TaskBinding,
        acting_user: &str,
    ) -> Option<ExternalStatus> {
        let subject = GithubPrSubject::from_destination_config(&binding.destination_config)?;

        let api = match self
            .auth()
            .client_for_live_status(
                acting_user,
                &GitHostRepo::github(subject.owner(), subject.repo()),
            )
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

        let variables = serde_json::json!({
            "owner": subject.owner(),
            "name": subject.repo(),
            "number": subject.pr_number(),
        });
        match pull_request_data::<LivePullRequest>(&api, LIVE_STATUS_QUERY, variables).await {
            Ok(pr) => Some(status_from_live_read(&pr)),
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
    use moatless_types::adapter::AdapterKind;
    use std::sync::Arc;

    use axum::http::{Method, StatusCode};

    use super::super::test_support::{StubResponse, start_github_api_stub};
    use super::*;
    use moatless_identity::auth::test_support::{StubProviderTokens, github_row};

    fn binding_with_config(config: serde_json::Value) -> TaskBinding {
        let now = chrono::Utc::now();
        TaskBinding {
            id: "b1".into(),
            task_id: "t1".into(),
            adapter_kind: AdapterKind::Github,
            subject_kind: "github_pr".into(),
            subject_key: "acme/web#7".into(),
            connection_id: "c1".into(),
            loop_id: None,
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
            watching: true,
        }
    }

    #[test]
    fn github_merged_beats_closed_and_draft() {
        let s = github_external_status(&PullRequestFields {
            state: "closed",
            merged: true,
            ..PullRequestFields::default()
        });
        assert_eq!(s.status.as_deref(), Some("merged"));
        assert_eq!(s.state_kind.as_deref(), Some("merged"));
    }

    #[test]
    fn github_closed_not_merged() {
        let s = github_external_status(&PullRequestFields {
            state: "closed",
            ..PullRequestFields::default()
        });
        assert_eq!(s.status.as_deref(), Some("closed"));
    }

    #[test]
    fn github_draft_when_open() {
        let s = github_external_status(&PullRequestFields {
            state: "open",
            draft: true,
            ..PullRequestFields::default()
        });
        assert_eq!(s.status.as_deref(), Some("draft"));
        assert_eq!(s.state_kind.as_deref(), Some("draft"));
    }

    #[test]
    fn github_plain_open() {
        let s = github_external_status(&PullRequestFields {
            state: "open",
            ..PullRequestFields::default()
        });
        assert_eq!(s.status.as_deref(), Some("open"));
    }

    #[test]
    fn the_blob_round_trips_base_and_head() {
        let s = github_external_status(&PullRequestFields {
            state: "open",
            title: Some("t"),
            url: Some("u"),
            base_ref: Some("main"),
            head_ref: Some("feature"),
            ..PullRequestFields::default()
        });
        let mut binding = binding_with_config(serde_json::json!({}));
        binding.external_status_raw = Some(s.raw);
        assert_eq!(
            status_base_head(&binding),
            Some(("main".to_string(), "feature".to_string()))
        );
    }

    #[test]
    fn a_blob_without_base_and_head_reads_incomplete() {
        // The shape written before this change: no baseRef, no headRef.
        let mut binding = binding_with_config(serde_json::json!({}));
        binding.external_status_raw = Some(serde_json::json!({
            "title": "t",
            "url": "u",
            "state": "open",
            "merged": false,
            "draft": false,
        }));
        assert!(status_base_head(&binding).is_none());
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

    fn live_read(state: &str, draft: bool, rollup: Option<&str>) -> LivePullRequest {
        serde_json::from_value(serde_json::json!({
            "title": "t",
            "url": "https://github.com/o/r/pull/1",
            "state": state,
            "isDraft": draft,
            "baseRefName": "main",
            "headRefName": "feature",
            "updatedAt": "2026-10-24T09:30:00Z",
            "commits": {"nodes": [
                {"commit": {"statusCheckRollup": rollup.map(|state| serde_json::json!({"state": state}))}}
            ]}
        }))
        .expect("a live read decodes")
    }

    #[test]
    fn a_rollup_state_maps_onto_t3s_checks_state() {
        assert_eq!(checks_state("SUCCESS"), Some("passing"));
        assert_eq!(checks_state("FAILURE"), Some("failing"));
        assert_eq!(checks_state("ERROR"), Some("failing"));
        assert_eq!(checks_state("PENDING"), Some("pending"));
        assert_eq!(checks_state("EXPECTED"), Some("pending"));
        assert_eq!(checks_state("NEUTRAL"), None);
    }

    #[test]
    fn a_live_read_carries_updated_at_and_the_checks_rollup() {
        let s = status_from_live_read(&live_read("OPEN", false, Some("PENDING")));
        assert_eq!(s.state_kind.as_deref(), Some("open"));
        assert_eq!(s.raw["updatedAt"], "2026-10-24T09:30:00Z");
        assert_eq!(s.raw["checksState"], "pending");
    }

    #[test]
    fn a_head_without_checks_reads_as_null_checks() {
        let s = status_from_live_read(&live_read("OPEN", false, None));
        assert_eq!(s.raw["checksState"], serde_json::Value::Null);
        assert!(s.raw.get("checksState").is_some());
    }

    #[test]
    fn a_live_read_keeps_merged_closed_and_draft_apart() {
        let merged = status_from_live_read(&live_read("MERGED", false, None));
        assert_eq!(merged.state_kind.as_deref(), Some("merged"));
        assert_eq!(merged.raw["state"], "closed");
        let closed = status_from_live_read(&live_read("CLOSED", false, None));
        assert_eq!(closed.state_kind.as_deref(), Some("closed"));
        let draft = status_from_live_read(&live_read("OPEN", true, None));
        assert_eq!(draft.state_kind.as_deref(), Some("draft"));
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
            (Method::POST, "/graphql") => StubResponse::json(
                StatusCode::OK,
                serde_json::json!({"data": {"repository": {"pullRequest": {
                    "title": "Characterize contracts",
                    "url": "https://github.com/soaplabs/repo/pull/42",
                    "state": "OPEN",
                    "isDraft": false,
                    "baseRefName": "main",
                    "headRefName": "feature",
                    "updatedAt": "2026-10-24T09:30:00Z",
                    "commits": {"nodes": [
                        {"commit": {"statusCheckRollup": {"state": "FAILURE"}}}
                    ]}
                }}}}),
            ),
            _ => StubResponse::not_found(uri),
        })
        .await;

        let provider = GithubPrAdapter::new(
            None,
            "https://app.example.com".to_string(),
            moatless_core::config::DEFAULT_GITHUB_API_BASE.to_string(),
            moatless_identity::git_host_tokens::seam_over(Arc::new(
                StubProviderTokens::with_user_row_for(
                    "owner-user",
                    github_row("manual", "owner-token"),
                ),
            )),
            moatless_core::config::HttpConfig::default().github,
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
        assert_eq!(status.raw["updatedAt"], "2026-10-24T09:30:00Z");
        assert_eq!(status.raw["checksState"], "failing");
        assert_eq!(status.raw["headRef"], "feature");

        // Every request below was provoked by this test: the stub is reached
        // only by the client this Adapter built against it.
        let recorded = stub.recorded_requests().await;
        let selected_status_token = Some("Bearer owner-token");
        let status_requests: Vec<_> = recorded.iter().collect();
        let observed: Vec<(&str, &str)> = status_requests
            .iter()
            .map(|request| (request.method.as_str(), request.uri.as_str()))
            .collect();
        assert_eq!(observed, vec![("POST", "/graphql")]);
        assert!(
            status_requests
                .iter()
                .all(|request| request.authorization.as_deref() == selected_status_token),
            "status refresh keeps its selected user token: {recorded:?}"
        );
    }
}
