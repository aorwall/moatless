//! GitHub REST API client for PR interactions.

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use tracing::warn;

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};

use crate::rejection::{graphql_rate_limited, rejection};
use moatless_types::http::HttpTimeouts;

/// Maximum comment length for GitHub's API (64 KB).
pub(super) const MAX_COMMENT_LENGTH: usize = 65_536;

/// A single reaction returned by the GitHub reactions API.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct ReactionInfo {
    pub(super) id: u64,
    pub(super) content: String,
    #[serde(default)]
    pub(super) user: Option<ReactionUser>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ReactionUser {
    pub(super) login: String,
}

/// GitHub PR metadata from the API.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct PrInfo {
    pub(super) number: u64,
    pub(super) title: String,
    pub(super) state: String,
    /// Whether the PR was merged. Distinguishes `merged` from a plain `closed`
    /// PR (GitHub reports both as `state = "closed"`). Absent on older
    /// payloads, hence optional.
    #[serde(default)]
    pub(super) merged: Option<bool>,
    /// Whether the PR is a draft.
    #[serde(default)]
    pub(super) draft: Option<bool>,
    /// Whether GitHub currently considers the PR mergeable.
    ///
    /// `None` until GitHub's background mergeability job finishes; the first
    /// `GET` on the PR is what triggers that job, so a fresh read is often
    /// `null`.
    #[serde(default)]
    pub(super) mergeable: Option<bool>,
    /// Coarse merge state (`clean`, `dirty`, `blocked`, `unknown`, …).
    ///
    /// Only present on the single-PR endpoint, never on list/webhook payloads.
    #[serde(default)]
    pub(super) mergeable_state: Option<String>,
    /// PR web URL (`html_url`).
    #[serde(default)]
    pub(super) html_url: Option<String>,
    pub(super) user: Option<PrInfoUser>,
    pub(super) head: PrRef,
    pub(super) base: PrRef,
    /// The rest are only on the single-PR endpoint.
    #[serde(default)]
    pub(super) body: Option<String>,
    #[serde(default)]
    pub(super) additions: Option<u64>,
    #[serde(default)]
    pub(super) deletions: Option<u64>,
    #[serde(default)]
    pub(super) changed_files: Option<u64>,
    #[serde(default)]
    pub(super) created_at: Option<String>,
    #[serde(default)]
    pub(super) updated_at: Option<String>,
    #[serde(default)]
    pub(super) merged_at: Option<String>,
    #[serde(default)]
    pub(super) closed_at: Option<String>,
}

impl PrInfo {
    #[expect(dead_code, reason = "kept with the concrete GitHub PR response model")]
    pub(super) fn author(&self) -> &str {
        self.user.as_ref().map_or("unknown", |u| u.login.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct PrInfoUser {
    pub(super) login: String,
    #[serde(default)]
    pub(super) avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct PrRef {
    #[serde(rename = "ref")]
    pub(super) ref_name: String,
    pub(super) sha: String,
    /// Absent when the head repository was deleted.
    #[serde(default)]
    pub(super) repo: Option<PrRepo>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct PrRepo {
    pub(super) full_name: String,
}

/// One changed file of a pull request, as `GET /pulls/{n}/files` lists it.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct PrFile {
    pub(super) filename: String,
    #[serde(default)]
    pub(super) previous_filename: Option<String>,
    /// `added`, `removed`, `modified`, `renamed`, `copied`, `changed` or
    /// `unchanged`.
    pub(super) status: String,
    /// The unified hunks, without file headers. Absent for a binary file or
    /// one too large for GitHub to diff.
    #[serde(default)]
    pub(super) patch: Option<String>,
}

/// Largest file [`GithubPrApi::file_text`] reads.
pub(super) const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

/// One commit as `GET /commits/{sha}` returns it, with one page of its files.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct CommitInfo {
    pub(super) sha: String,
    #[serde(default)]
    pub(super) parents: Vec<CommitParent>,
    #[serde(default)]
    pub(super) files: Vec<PrFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CommitParent {
    pub(super) sha: String,
}

/// A pull request as returned by `GET /commits/{sha}/pulls` — the minimal
/// subset needed to identify the merged PR that produced a commit.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct CommitPr {
    pub(super) number: u64,
    #[expect(
        dead_code,
        reason = "deserialized GitHub field retained for endpoint parity"
    )]
    pub(super) state: String,
    /// Set (non-null) once the PR has been merged. Distinguishes a merged PR
    /// from an open PR that merely has this commit on its head branch.
    #[serde(default)]
    pub(super) merged_at: Option<String>,
}

impl CommitPr {
    /// A PR is merged when GitHub stamped `merged_at`.
    pub(super) fn is_merged(&self) -> bool {
        self.merged_at.is_some()
    }
}

/// A workflow run as returned by `GET /actions/runs`: the subset that says
/// whether it is still running and how it ended.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct WorkflowRunSummary {
    pub(super) name: String,
    pub(super) status: String,
    /// `None` until the run completes.
    #[serde(default)]
    pub(super) conclusion: Option<String>,
}

/// One file-scoped finding attached to a check run, as returned by
/// `GET /check-runs/{id}/annotations`.
///
/// A GitHub Actions job reports a failure here — via `::error file=…,line=…::`
/// — and leaves `output.summary` empty, so the check run's webhook payload
/// carries no trace of what failed beyond a count.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct CheckAnnotation {
    /// Repository-relative path, or `.github` when the failure is the step
    /// itself rather than a line of code.
    #[serde(default)]
    pub(super) path: String,
    #[serde(default)]
    pub(super) start_line: Option<u64>,
    #[serde(default)]
    pub(super) end_line: Option<u64>,
    /// `failure`, `warning`, or `notice`.
    #[serde(default)]
    pub(super) annotation_level: Option<String>,
    #[serde(default)]
    pub(super) message: Option<String>,
}

/// GitHub REST API client for PR operations.
pub(super) struct GithubPrApi {
    client: Client,
    token: String,
    base_url: String,
}

/// The client every [`GithubPrApi`] shares, bounded by `timeouts`.
#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
)]
pub(super) fn http_client(timeouts: HttpTimeouts) -> Client {
    Client::builder()
        .connect_timeout(timeouts.connect)
        .timeout(timeouts.total)
        .build()
        .expect("the TLS backend initialises")
}

/// The GraphQL endpoint beside the REST root `base_url`: Enterprise Server
/// serves REST at `/api/v3` and GraphQL at `/api/graphql`.
pub(super) fn graphql_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    match base.strip_suffix("/v3") {
        Some(api_root) => format!("{api_root}/graphql"),
        None => format!("{base}/graphql"),
    }
}

impl GithubPrApi {
    /// A client for `token` against the REST root `base_url`.
    pub(super) fn new(client: Client, token: &str, base_url: impl Into<String>) -> Self {
        Self {
            client,
            token: token.to_string(),
            base_url: base_url.into(),
        }
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "moatless-vibe")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    /// Run a GraphQL `query` with `variables`, returning its `data`. A
    /// response carrying `errors` is an error even when `data` is partial.
    pub(super) async fn graphql(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        let url = graphql_url(&self.base_url);
        let resp = self
            .request(reqwest::Method::POST, &url)
            .json(&serde_json::json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub GraphQL", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub GraphQL", status, &headers, &text));
        }
        let headers = resp.headers().clone();
        let mut body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("GitHub GraphQL", e))?;
        // GitHub answers a per-node failure (a SAML or IP allow-list
        // `FORBIDDEN`) with `errors` beside usable `data`, so only an answer
        // without `data` fails.
        let data = body["data"].take();
        if let Some(errors) = body.get("errors").filter(|errors| !errors.is_null()) {
            if data.is_null() {
                if graphql_rate_limited(errors) {
                    return Err(SharedAdapterError::RateLimited {
                        detail: format!("GitHub GraphQL: {errors}"),
                        retry_at: moatless_core::github_client::github_retry_at(
                            &headers,
                            chrono::Utc::now(),
                        ),
                    }
                    .into());
                }
                return Err(
                    SharedAdapterError::AdapterApi(format!("GitHub GraphQL: {errors}")).into(),
                );
            }
            tracing::warn!(%errors, "GitHub GraphQL answered with partial data");
        }
        Ok(data)
    }

    /// Post a comment on a PR (via the Issues API).
    pub(super) async fn post_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        body: &str,
    ) -> Result<u64, AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/issues/{pr_number}/comments",
            self.base_url
        );

        let body = crate::text::truncate_on_char_boundary(body, MAX_COMMENT_LENGTH);

        let resp = self
            .request(reqwest::Method::POST, &url)
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub post_comment", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub post_comment error");
            return Err(rejection("GitHub post_comment", status, &headers, &text));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("GitHub post_comment", e))?;

        let comment_id = data
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        Ok(comment_id)
    }

    /// Create a pull request. Returns the raw GitHub PR payload so callers can
    /// extract the fields they need (`number`, `html_url`, ...).
    #[allow(clippy::too_many_arguments)] // reason: mirrors the GitHub create-PR parameter set
    pub(super) async fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
        draft: bool,
    ) -> Result<serde_json::Value, AdapterError> {
        let url = format!("{}/repos/{owner}/{repo}/pulls", self.base_url);

        let resp = self
            .request(reqwest::Method::POST, &url)
            .json(&serde_json::json!({
                "title": title,
                "body": body,
                "head": head,
                "base": base,
                "draft": draft,
            }))
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub create_pr", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub create_pr error");
            return Err(rejection("GitHub create_pr", status, &headers, &text));
        }

        resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub create_pr", e).into()
        })
    }

    /// Fetch a repository's default branch name.
    pub(super) async fn get_repo_default_branch(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<String, AdapterError> {
        let url = format!("{}/repos/{owner}/{repo}", self.base_url);

        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub get_repo", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub get_repo", status, &headers, &text));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("GitHub get_repo", e))?;
        data.get("default_branch")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi("GitHub get_repo: missing default_branch".into())
                    .into()
            })
    }

    /// List pull requests associated with a commit sha.
    ///
    /// Used to map a main-branch CI run (no PR reference in its webhook) back
    /// to the PR whose merge produced the commit.
    pub(super) async fn get_pulls_for_commit(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<CommitPr>, AdapterError> {
        let url = format!("{}/repos/{owner}/{repo}/commits/{sha}/pulls", self.base_url);

        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_unreachable("GitHub get_pulls_for_commit", e)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection(
                "GitHub get_pulls_for_commit",
                status,
                &headers,
                &text,
            ));
        }

        resp.json::<Vec<CommitPr>>().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub get_pulls_for_commit", e).into()
        })
    }

    /// List the workflow runs a push of `sha` started, latest attempt each.
    ///
    /// One page of 100: a commit starting more workflows than that reads as
    /// not yet passed, so the pipeline notice is withheld rather than wrong.
    pub(super) async fn list_push_runs_for_commit(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<WorkflowRunSummary>, AdapterError> {
        #[derive(Deserialize)]
        struct Page {
            total_count: usize,
            workflow_runs: Vec<WorkflowRunSummary>,
        }
        let url = format!("{}/repos/{owner}/{repo}/actions/runs", self.base_url);
        let resp = self
            .request(reqwest::Method::GET, &url)
            .query(&[("head_sha", sha), ("event", "push"), ("per_page", "100")])
            .send()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_unreachable("GitHub list_push_runs_for_commit", e)
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection(
                "GitHub list_push_runs_for_commit",
                status,
                &headers,
                &text,
            ));
        }
        let page: Page = resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub list_push_runs_for_commit", e)
        })?;
        let mut runs = page.workflow_runs;
        if page.total_count > runs.len() {
            runs.push(WorkflowRunSummary {
                name: "(beyond the first page)".into(),
                status: "unknown".into(),
                conclusion: None,
            });
        }
        Ok(runs)
    }

    /// List the annotations a completed check run left on the diff.
    ///
    /// One page only: the caller renders far fewer than a page holds, and the
    /// notice this feeds is a pointer at the failure rather than a full report.
    pub(super) async fn get_check_run_annotations(
        &self,
        owner: &str,
        repo: &str,
        check_run_id: u64,
    ) -> Result<Vec<CheckAnnotation>, AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/check-runs/{check_run_id}/annotations",
            self.base_url
        );

        let resp = self
            .request(reqwest::Method::GET, &url)
            .query(&[("per_page", "100")])
            .send()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_unreachable("GitHub get_check_run_annotations", e)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection(
                "GitHub get_check_run_annotations",
                status,
                &headers,
                &text,
            ));
        }

        resp.json::<Vec<CheckAnnotation>>().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub get_check_run_annotations", e)
                .into()
        })
    }

    /// The open pull request whose head branch is `head`, if any.
    ///
    /// GitHub filters `head` as `{owner}:{branch}`, so a fork's like-named
    /// branch is never mistaken for this repo's; a head branch has at most one
    /// open PR, so the first match is the answer.
    pub(super) async fn find_open_pr_for_head(
        &self,
        owner: &str,
        repo: &str,
        head: &str,
    ) -> Result<Option<PrInfo>, AdapterError> {
        let url = format!("{}/repos/{owner}/{repo}/pulls", self.base_url);
        let head_filter = format!("{owner}:{head}");

        let resp = self
            .request(reqwest::Method::GET, &url)
            .query(&[
                ("head", head_filter.as_str()),
                ("state", "open"),
                ("per_page", "1"),
            ])
            .send()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_unreachable("GitHub find_open_pr_for_head", e)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection(
                "GitHub find_open_pr_for_head",
                status,
                &headers,
                &text,
            ));
        }

        let prs: Vec<PrInfo> = resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub find_open_pr_for_head", e)
        })?;
        Ok(prs.into_iter().next())
    }

    /// Reply to a PR review comment (threaded review reply).
    ///
    /// Returns the new review comment id.
    pub(super) async fn reply_to_review_comment(
        &self,
        owner: &str,
        repo: &str,
        pull_number: u64,
        comment_id: u64,
        body: &str,
    ) -> Result<u64, AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies",
            self.base_url
        );

        let body = crate::text::truncate_on_char_boundary(body, MAX_COMMENT_LENGTH);

        let resp = self
            .request(reqwest::Method::POST, &url)
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_unreachable("GitHub reply_to_review_comment", e)
            })?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            // GitHub answers 404 alike for a deleted comment, a conversation
            // comment id and a repository the token cannot see, so the message
            // names all three rather than claiming one.
            warn!(%comment_id, "GitHub reply_to_review_comment target not found");
            return Err(SharedAdapterError::NotFound(format!(
                "GitHub has no review comment {comment_id} on {owner}/{repo}#{pull_number} \
                 this token can reply to: it may have been deleted, be a conversation \
                 comment, or be on a pull request the token cannot read. Reply to a current \
                 review comment id, or post a new comment on #{pull_number}"
            ))
            .into());
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub reply_to_review_comment error");
            return Err(rejection(
                "GitHub reply_to_review_comment",
                status,
                &headers,
                &text,
            ));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub reply_to_review_comment", e)
        })?;

        data.get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi("GitHub reply_to_review_comment: missing id".into())
                    .into()
            })
    }

    /// Get PR metadata (title, body, author, state, branches).
    pub(super) async fn get_pr(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<PrInfo, AdapterError> {
        let url = format!("{}/repos/{owner}/{repo}/pulls/{pr_number}", self.base_url);

        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub get_pr", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub get_pr", status, &headers, &text));
        }

        resp.json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("GitHub get_pr", e).into())
    }

    /// One page of a pull request's changed files, `per_page` at a time.
    pub(super) async fn list_pr_files(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<PrFile>, AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls/{pr_number}/files?per_page={per_page}&page={page}",
            self.base_url
        );
        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub list_pr_files", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub list_pr_files", status, &headers, &text));
        }
        resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub list_pr_files", e).into()
        })
    }

    /// One commit with page `page` of its changed files, `per_page` at a time.
    pub(super) async fn get_commit(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
        page: u32,
        per_page: u32,
    ) -> Result<CommitInfo, AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/commits/{sha}?per_page={per_page}&page={page}",
            self.base_url
        );
        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub get_commit", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub get_commit", status, &headers, &text));
        }
        resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub get_commit", e).into()
        })
    }

    /// The commit `base` and `head` last share: the old side of a pull
    /// request's three-dot diff.
    pub(super) async fn merge_base(
        &self,
        owner: &str,
        repo: &str,
        base: &str,
        head: &str,
    ) -> Result<String, AdapterError> {
        #[derive(Deserialize)]
        struct Comparison {
            merge_base_commit: CommitParent,
        }
        // `per_page=1` keeps the commits and files GitHub lists alongside small.
        let url = format!(
            "{}/repos/{owner}/{repo}/compare/{base}...{head}?per_page=1",
            self.base_url
        );
        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub merge_base", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub merge_base", status, &headers, &text));
        }
        let comparison: Comparison = resp
            .json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("GitHub merge_base", e))?;
        Ok(comparison.merge_base_commit.sha)
    }

    /// `path` as it is at `revision`, as text. Fails for a file that is
    /// binary or larger than [`MAX_FILE_BYTES`].
    pub(super) async fn file_text(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        revision: &str,
    ) -> Result<String, AdapterError> {
        let invalid = |detail: String| AdapterError::from(SharedAdapterError::AdapterApi(detail));
        let mut url = reqwest::Url::parse(&self.base_url)
            .map_err(|e| invalid(format!("GitHub base URL is not a URL: {e}")))?;
        url.path_segments_mut()
            .map_err(|()| invalid("GitHub base URL cannot take a path".to_string()))?
            .pop_if_empty()
            .extend(["repos", owner, repo, "contents"])
            .extend(path.split('/'));
        url.query_pairs_mut().append_pair("ref", revision);
        let mut resp = self
            .request(reqwest::Method::GET, url.as_str())
            .header("Accept", "application/vnd.github.raw+json")
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub file_text", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub file_text", status, &headers, &text));
        }
        let too_large = || invalid(format!("{path} is too large to show"));
        // Read in chunks so a file of up to GitHub's 100 MB is refused at the
        // cap rather than first held whole in memory.
        if resp
            .content_length()
            .is_some_and(|length| length > MAX_FILE_BYTES as u64)
        {
            return Err(too_large());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("GitHub file_text", e))?
        {
            if bytes.len() + chunk.len() > MAX_FILE_BYTES {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.contains(&0) {
            return Err(invalid(format!("{path} is a binary file")));
        }
        String::from_utf8(bytes).map_err(|_| invalid(format!("{path} is not UTF-8 text")))
    }

    /// Fetch PR metadata, polling until GitHub has computed the mergeable state
    /// (or a small retry budget is exhausted).
    ///
    /// Never errors on timeout: if mergeability never settles, the last
    /// (possibly `unknown`) `PrInfo` is returned and callers treat `unknown` as
    /// inconclusive.
    pub(super) async fn get_pr_mergeable(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<PrInfo, AdapterError> {
        /// Retries after the initial read (which usually returns `unknown`).
        const MERGEABLE_POLL_RETRIES: usize = 3;
        /// Backoff between reads while GitHub computes mergeability.
        const MERGEABLE_POLL_DELAY: Duration = Duration::from_secs(2);

        let mut pr = self.get_pr(owner, repo, pr_number).await?;
        for _ in 0..MERGEABLE_POLL_RETRIES {
            let settled = pr.mergeable.is_some()
                && pr
                    .mergeable_state
                    .as_deref()
                    .is_some_and(|s| s != "unknown");
            // A closed/merged PR never gets a mergeable state — stop early.
            if settled || !pr.state.eq_ignore_ascii_case("open") {
                return Ok(pr);
            }
            tokio::time::sleep(MERGEABLE_POLL_DELAY).await;
            pr = self.get_pr(owner, repo, pr_number).await?;
        }
        Ok(pr)
    }

    /// Create a reaction on an issue comment (also covers PR conversation
    /// comments, since they share the issues comments API).
    ///
    /// Returns the GitHub reaction id.
    pub(super) async fn create_issue_comment_reaction(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        content: &str,
    ) -> Result<u64, AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/issues/comments/{comment_id}/reactions",
            self.base_url
        );
        self.create_reaction(&url, content).await
    }

    /// Create a reaction on a PR review (line) comment.
    pub(super) async fn create_review_comment_reaction(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        content: &str,
    ) -> Result<u64, AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls/comments/{comment_id}/reactions",
            self.base_url
        );
        self.create_reaction(&url, content).await
    }

    async fn create_reaction(&self, url: &str, content: &str) -> Result<u64, AdapterError> {
        let resp = self
            .request(reqwest::Method::POST, url)
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub create_reaction", e))?;

        let status = resp.status();
        // GitHub returns 200 (existing) or 201 (created); both contain the reaction body.
        if !status.is_success() {
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub create_reaction error");
            return Err(rejection("GitHub create_reaction", status, &headers, &text));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub create_reaction", e)
        })?;
        let reaction_id = data
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi("GitHub create_reaction: missing id".into())
            })?;
        Ok(reaction_id)
    }

    /// List reactions on an issue comment.
    pub(super) async fn list_issue_comment_reactions(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
    ) -> Result<Vec<ReactionInfo>, AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/issues/comments/{comment_id}/reactions",
            self.base_url
        );
        self.list_reactions(&url).await
    }

    /// List reactions on a PR review comment.
    pub(super) async fn list_review_comment_reactions(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
    ) -> Result<Vec<ReactionInfo>, AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls/comments/{comment_id}/reactions",
            self.base_url
        );
        self.list_reactions(&url).await
    }

    async fn list_reactions(&self, url: &str) -> Result<Vec<ReactionInfo>, AdapterError> {
        let resp = self
            .request(reqwest::Method::GET, url)
            .query(&[("per_page", "100")])
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub list_reactions", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub list_reactions", status, &headers, &text));
        }

        resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub list_reactions", e).into()
        })
    }

    /// Delete a reaction from an issue comment by reaction id.
    pub(super) async fn delete_issue_comment_reaction(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        reaction_id: u64,
    ) -> Result<(), AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/issues/comments/{comment_id}/reactions/{reaction_id}",
            self.base_url
        );
        self.delete_reaction(&url).await
    }

    /// Delete a reaction from a PR review comment by reaction id.
    pub(super) async fn delete_review_comment_reaction(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        reaction_id: u64,
    ) -> Result<(), AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls/comments/{comment_id}/reactions/{reaction_id}",
            self.base_url
        );
        self.delete_reaction(&url).await
    }

    async fn delete_reaction(&self, url: &str) -> Result<(), AdapterError> {
        let resp = self
            .request(reqwest::Method::DELETE, url)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub delete_reaction", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub delete_reaction", status, &headers, &text));
        }
        Ok(())
    }

    /// Look up the authenticated bot's GitHub login. Used to filter reactions
    /// when removing.
    pub(super) async fn authenticated_login(&self) -> Result<String, AdapterError> {
        let resp = self
            .request(reqwest::Method::GET, &format!("{}/user", self.base_url))
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub user", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub user", status, &headers, &text));
        }
        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("GitHub user", e))?;
        data.get("login")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi("GitHub user: missing login".into()).into()
            })
    }
}

#[cfg(test)]
mod error_source_tests {
    use std::error::Error as _;

    use axum::http::StatusCode;

    use super::*;
    use moatless_adapter_model::test_support::{UNREACHABLE_BASE, stub_provider};

    #[tokio::test]
    async fn unreachable_github_keeps_transport_source() {
        let api = GithubPrApi::new(test_client(), "ghp_test", UNREACHABLE_BASE);

        let err = api
            .post_comment("acme", "widgets", 7, "hi")
            .await
            .expect_err("dead loopback port must fail");

        let AdapterError::Shared(SharedAdapterError::ProviderUnreachable {
            ref operation,
            ref source,
        }) = err
        else {
            panic!("expected ProviderUnreachable, got {err:?}");
        };
        assert_eq!(operation, "GitHub post_comment");
        assert!(source.is_connect() || source.is_request(), "{source}");
        assert!(err.source().is_some(), "cause must stay reachable");
    }

    #[tokio::test]
    async fn undecodable_github_body_keeps_decode_source() {
        let base = stub_provider(StatusCode::CREATED, "<html>not json</html>").await;
        let api = GithubPrApi::new(test_client(), "ghp_test", base);

        let err = api
            .post_comment("acme", "widgets", 7, "hi")
            .await
            .expect_err("html body must not decode");

        let AdapterError::Shared(SharedAdapterError::ProviderResponseInvalid {
            ref operation, ..
        }) = err
        else {
            panic!("expected ProviderResponseInvalid, got {err:?}");
        };
        assert_eq!(operation, "GitHub post_comment");
        assert!(err.source().is_some(), "cause must stay reachable");
    }

    /// GitHub answering 403 without a rate-limit mark is a permission
    /// refusal, synthesized rather than wrapped.
    #[tokio::test]
    async fn github_rejection_stays_a_synthesized_message() {
        let base = stub_provider(StatusCode::FORBIDDEN, r#"{"message":"Bad credentials"}"#).await;
        let api = GithubPrApi::new(test_client(), "ghp_test", base);

        let err = api
            .post_comment("acme", "widgets", 7, "hi")
            .await
            .expect_err("403 must be an error");

        assert!(
            matches!(
                err,
                AdapterError::Shared(SharedAdapterError::AccessDenied(_))
            ),
            "got {err:?}"
        );
        assert!(err.source().is_none());
    }

    /// The reset GitHub sends on a spent quota reaches the error, so the
    /// delivery seam can schedule at it.
    #[tokio::test]
    async fn a_spent_quota_carries_its_reset_to_the_error() {
        use axum::Router;
        use axum::response::IntoResponse;
        use axum::routing::get;

        async fn spent() -> axum::response::Response {
            (
                StatusCode::FORBIDDEN,
                [
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", "1793002200"),
                ],
                r#"{"message":"API rate limit exceeded for user ID 609489."}"#,
            )
                .into_response()
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route("/repos/acme/widgets/pulls/42", get(spent));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let api = GithubPrApi::new(test_client(), "ghp_test", format!("http://{address}"));
        let err = api
            .get_pr("acme", "widgets", 42)
            .await
            .expect_err("a spent quota is an error");
        let AdapterError::Shared(SharedAdapterError::RateLimited { retry_at, .. }) = err else {
            panic!("expected RateLimited, got {err:?}");
        };
        assert_eq!(retry_at, chrono::DateTime::from_timestamp(1_793_002_200, 0));
    }
}

#[cfg(test)]
mod request_budget_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::extract::State;
    use axum::routing::get;
    use axum::{Json, Router};

    use super::*;

    #[tokio::test]
    async fn unresolved_mergeability_stops_after_four_pr_reads() {
        async fn unknown_mergeability(
            State(reads): State<Arc<AtomicUsize>>,
        ) -> Json<serde_json::Value> {
            reads.fetch_add(1, Ordering::SeqCst);
            Json(serde_json::json!({
                "number": 42,
                "title": "Add widgets",
                "body": "Body",
                "state": "open",
                "merged": false,
                "mergeable": serde_json::Value::Null,
                "mergeable_state": "unknown",
                "draft": false,
                "html_url": "https://github.com/acme/widgets/pull/42",
                "user": {"login": "octocat"},
                "head": {"ref": "feature", "sha": "abc1234"},
                "base": {"ref": "main", "sha": "def4567"}
            }))
        }

        let reads = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind GitHub stub");
        let address = listener.local_addr().expect("GitHub stub address");
        let app = Router::new()
            .route("/repos/acme/widgets/pulls/42", get(unknown_mergeability))
            .with_state(reads.clone());
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("GitHub stub serves");
        });

        let api = GithubPrApi::new(test_client(), "ghp_test", format!("http://{address}"));
        let pr = api
            .get_pr_mergeable("acme", "widgets", 42)
            .await
            .expect("the final unresolved PR is returned");

        assert_eq!(pr.mergeable_state.as_deref(), Some("unknown"));
        assert_eq!(
            reads.load(Ordering::SeqCst),
            4,
            "one initial read plus at most three mergeability retries"
        );
    }
}

#[cfg(test)]
mod find_open_pr_tests {
    use axum::extract::{Query, State};
    use axum::routing::get;
    use axum::{Json, Router};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// The head filter GitHub was asked for, plus a one-element result, so the
    /// test can assert both the `{owner}:{branch}` filter and that the first
    /// match is returned.
    #[tokio::test]
    async fn find_open_pr_for_head_filters_by_owner_and_head_and_takes_the_first() {
        let seen_head = Arc::new(Mutex::new(String::new()));

        async fn pulls(
            State(seen): State<Arc<Mutex<String>>>,
            Query(params): Query<HashMap<String, String>>,
        ) -> Json<serde_json::Value> {
            *seen.lock().expect("lock") = params.get("head").cloned().unwrap_or_default();
            Json(serde_json::json!([{
                "number": 7,
                "title": "Existing",
                "body": null,
                "state": "open",
                "html_url": "https://github.com/acme/widgets/pull/7",
                "user": {"login": "octocat"},
                "head": {"ref": "feature", "sha": "abc"},
                "base": {"ref": "main", "sha": "def"},
            }]))
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind GitHub stub");
        let address = listener.local_addr().expect("stub address");
        let app = Router::new()
            .route("/repos/acme/widgets/pulls", get(pulls))
            .with_state(seen_head.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("stub serves");
        });

        let api = GithubPrApi::new(test_client(), "ghp_test", format!("http://{address}"));
        let found = api
            .find_open_pr_for_head("acme", "widgets", "feature")
            .await
            .expect("the lookup succeeds")
            .expect("one open PR");

        assert_eq!(found.number, 7);
        assert_eq!(found.head.ref_name, "feature");
        assert_eq!(*seen_head.lock().expect("lock"), "acme:feature");
    }

    #[tokio::test]
    async fn find_open_pr_for_head_is_none_when_the_branch_has_no_open_pr() {
        async fn empty() -> Json<serde_json::Value> {
            Json(serde_json::json!([]))
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind GitHub stub");
        let address = listener.local_addr().expect("stub address");
        let app = Router::new().route("/repos/o/r/pulls", get(empty));
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("stub serves");
        });

        let api = GithubPrApi::new(test_client(), "ghp_test", format!("http://{address}"));
        let found = api
            .find_open_pr_for_head("o", "r", "feature")
            .await
            .expect("the lookup succeeds");
        assert!(found.is_none());
    }
}

#[cfg(test)]
mod serde_tests {
    use super::*;

    #[test]
    fn pr_info_deserializes_mergeable_fields() {
        let pr: PrInfo = serde_json::from_value(serde_json::json!({
            "number": 1,
            "title": "t",
            "body": null,
            "state": "open",
            "mergeable": false,
            "mergeable_state": "dirty",
            "user": {"login": "alice"},
            "head": {"ref": "feat", "sha": "abc"},
            "base": {"ref": "main", "sha": "def"},
        }))
        .expect("PrInfo parses mergeable fields");
        assert_eq!(pr.mergeable, Some(false));
        assert_eq!(pr.mergeable_state.as_deref(), Some("dirty"));

        let pr2: PrInfo = serde_json::from_value(serde_json::json!({
            "number": 2,
            "title": "t",
            "body": null,
            "state": "open",
            "user": {"login": "bob"},
            "head": {"ref": "h", "sha": "s"},
            "base": {"ref": "b", "sha": "s2"},
        }))
        .expect("PrInfo parses without mergeable fields");
        assert_eq!(pr2.mergeable, None);
        assert_eq!(pr2.mergeable_state, None);
    }

    #[test]
    fn commit_pr_merged_detection() {
        let merged: CommitPr = serde_json::from_value(serde_json::json!({
            "number": 344,
            "state": "closed",
            "merged_at": "2026-07-17T04:00:00Z",
        }))
        .expect("CommitPr parses");
        assert!(merged.is_merged());
        assert_eq!(merged.number, 344);

        let open: CommitPr =
            serde_json::from_value(serde_json::json!({ "number": 7, "state": "open" }))
                .expect("CommitPr parses without merged_at");
        assert!(!open.is_merged());
    }
}

#[cfg(test)]
mod review_reply_tests {
    use axum::http::{Method, StatusCode};
    use axum::response::IntoResponse;

    use super::*;
    use crate::test_support::{StubResponse, start_github_api_stub};

    async fn reply_answered_with(response: fn() -> StubResponse) -> Result<u64, AdapterError> {
        let stub = start_github_api_stub(move |method, path, uri| {
            if method == Method::POST && path == "/repos/acme/widgets/pulls/7/comments/55/replies" {
                response()
            } else {
                StubResponse::not_found(uri)
            }
        })
        .await;
        let api = GithubPrApi::new(test_client(), "ghp_test", stub.base_url().to_string());
        api.reply_to_review_comment("acme", "widgets", 7, 55, "hi")
            .await
    }

    #[tokio::test]
    async fn a_reply_returns_the_new_comment_id() {
        let id = reply_answered_with(|| {
            StubResponse::json(StatusCode::CREATED, serde_json::json!({"id": 102}))
        })
        .await
        .expect("201 with an id is a posted reply");
        assert_eq!(id, 102);
    }

    #[tokio::test]
    async fn a_missing_review_comment_is_a_not_found_rejection_not_a_bad_gateway() {
        let err = reply_answered_with(|| {
            StubResponse::json(
                StatusCode::NOT_FOUND,
                serde_json::json!({"message": "private provider detail"}),
            )
        })
        .await
        .expect_err("404 must fail the reply");

        assert!(
            matches!(err, AdapterError::Shared(SharedAdapterError::NotFound(_))),
            "got {err:?}"
        );
        let message = err.to_string();
        assert!(
            message.contains("review comment 55 on acme/widgets#7"),
            "{message}"
        );
        assert!(!message.contains("private provider detail"), "{message}");
        assert_eq!(err.into_response().status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn provider_and_credential_failures_still_render_as_a_bad_gateway() {
        for status in [StatusCode::INTERNAL_SERVER_ERROR, StatusCode::UNAUTHORIZED] {
            let stub = start_github_api_stub(move |_method, _path, _uri| {
                StubResponse::json(status, serde_json::json!({"message": "rejected"}))
            })
            .await;
            let api = GithubPrApi::new(test_client(), "ghp_test", stub.base_url().to_string());
            let err = api
                .reply_to_review_comment("acme", "widgets", 7, 55, "hi")
                .await
                .expect_err("a rejection must fail the reply");

            assert!(
                matches!(err, AdapterError::Shared(SharedAdapterError::AdapterApi(_))),
                "{status}: got {err:?}"
            );
            assert_eq!(err.into_response().status(), StatusCode::BAD_GATEWAY);
        }
    }

    #[tokio::test]
    async fn a_reply_without_an_id_still_renders_as_a_bad_gateway() {
        let err =
            reply_answered_with(|| StubResponse::json(StatusCode::CREATED, serde_json::json!({})))
                .await
                .expect_err("a reply with no id is unusable");
        assert_eq!(err.into_response().status(), StatusCode::BAD_GATEWAY);
    }
}

#[cfg(test)]
mod check_annotation_tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::test_support::{StubResponse, start_github_api_stub};

    #[tokio::test]
    async fn annotations_are_read_from_the_check_run_endpoint() {
        let stub = start_github_api_stub(|_method, path, uri| {
            if path == "/repos/acme/widgets/check-runs/31/annotations" {
                StubResponse::json(
                    StatusCode::OK,
                    serde_json::json!([{
                        "path": "crates/core/src/lib.rs",
                        "start_line": 10,
                        "end_line": 12,
                        "annotation_level": "failure",
                        "message": "comment block is 7 lines",
                    }]),
                )
            } else {
                StubResponse::not_found(uri)
            }
        })
        .await;
        let api = GithubPrApi::new(test_client(), "ghp_test", stub.base_url().to_string());

        let annotations = api
            .get_check_run_annotations("acme", "widgets", 31)
            .await
            .expect("the stub answers with a list");

        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].path, "crates/core/src/lib.rs");
        assert_eq!(annotations[0].start_line, Some(10));
        assert_eq!(annotations[0].end_line, Some(12));
        assert_eq!(annotations[0].annotation_level.as_deref(), Some("failure"));

        let recorded = stub.recorded_requests().await;
        assert_eq!(recorded.len(), 1);
        assert!(
            recorded[0].uri.contains("per_page=100"),
            "got {}",
            recorded[0].uri
        );
    }

    #[tokio::test]
    async fn a_rejected_annotations_read_is_an_error_not_an_empty_list() {
        let stub = start_github_api_stub(|_method, _path, _uri| {
            StubResponse::json(
                StatusCode::FORBIDDEN,
                serde_json::json!({"message": "Resource not accessible"}),
            )
        })
        .await;
        let api = GithubPrApi::new(test_client(), "ghp_test", stub.base_url().to_string());

        let err = api
            .get_check_run_annotations("acme", "widgets", 31)
            .await
            .expect_err("403 must not read as zero annotations");
        assert!(
            matches!(
                err,
                AdapterError::Shared(SharedAdapterError::AccessDenied(_))
            ),
            "got {err:?}"
        );
    }
}

#[cfg(test)]
mod file_text_tests {
    use axum::Router;
    use axum::extract::RawQuery;
    use axum::http::Uri;
    use axum::routing::get;

    use super::*;

    async fn serve() -> GithubPrApi {
        async fn contents(uri: Uri, RawQuery(query): RawQuery) -> Vec<u8> {
            match (uri.path(), query.as_deref()) {
                ("/repos/acme/app/contents/docs/a%20b.md", Some("ref=abc123")) => b"hello".to_vec(),
                ("/repos/acme/app/contents/logo.png", _) => vec![0x89, 0, 1],
                ("/repos/acme/app/contents/big.txt", _) => vec![b'a'; MAX_FILE_BYTES + 1],
                _ => b"unexpected".to_vec(),
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        async fn compare(uri: Uri) -> axum::Json<serde_json::Value> {
            assert_eq!(uri.path(), "/repos/acme/app/compare/main1...head1");
            axum::Json(serde_json::json!({ "merge_base_commit": { "sha": "fork1" } }))
        }
        let app = Router::new()
            .route("/repos/acme/app/contents/{*path}", get(contents))
            .route("/repos/acme/app/compare/{range}", get(compare));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        GithubPrApi::new(test_client(), "ghp_test", format!("http://{address}"))
    }

    #[tokio::test]
    async fn a_file_is_read_at_its_revision_with_its_path_escaped() {
        let api = serve().await;
        let text = api
            .file_text("acme", "app", "docs/a b.md", "abc123")
            .await
            .unwrap();
        assert_eq!(text, "hello");
    }

    #[tokio::test]
    async fn a_binary_file_is_refused() {
        let api = serve().await;
        let error = api
            .file_text("acme", "app", "logo.png", "abc123")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("binary"), "{error}");
    }

    #[tokio::test]
    async fn the_merge_base_is_read_from_the_comparison() {
        let api = serve().await;
        let base = api
            .merge_base("acme", "app", "main1", "head1")
            .await
            .unwrap();
        assert_eq!(base, "fork1");
    }

    #[tokio::test]
    async fn a_file_over_the_cap_is_refused() {
        let api = serve().await;
        let error = api
            .file_text("acme", "app", "big.txt", "abc123")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("too large"), "{error}");
    }
}

#[cfg(test)]
fn test_client() -> Client {
    http_client(moatless_core::config::HttpConfig::default().github)
}
