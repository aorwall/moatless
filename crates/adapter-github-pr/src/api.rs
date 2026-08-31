//! GitHub REST API client for PR interactions.

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use tracing::warn;

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};

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
    /// Whether GitHub currently considers the PR mergeable. `None` until the
    /// background mergeability job finishes — GitHub computes this lazily and
    /// the first `GET` on the PR is what kicks the computation off, so a fresh
    /// read is often `null`.
    #[serde(default)]
    pub(super) mergeable: Option<bool>,
    /// Coarse merge state: `clean`, `dirty` (has conflicts), `blocked`,
    /// `behind`, `unstable`, `has_hooks`, `draft`, or `unknown` (not yet
    /// computed). Only present on the single-PR endpoint, never on list/webhook
    /// payloads.
    #[serde(default)]
    pub(super) mergeable_state: Option<String>,
    /// PR web URL (`html_url`).
    #[serde(default)]
    pub(super) html_url: Option<String>,
    #[expect(
        dead_code,
        reason = "deserialized GitHub field retained for endpoint parity"
    )]
    pub(super) user: Option<PrInfoUser>,
    pub(super) head: PrRef,
    pub(super) base: PrRef,
}

impl PrInfo {
    #[expect(dead_code, reason = "kept with the concrete GitHub PR response model")]
    pub(super) fn author(&self) -> &str {
        self.user.as_ref().map_or("unknown", |u| u.login.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct PrInfoUser {
    #[expect(
        dead_code,
        reason = "deserialized GitHub field retained for endpoint parity"
    )]
    pub(super) login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct PrRef {
    #[serde(rename = "ref")]
    pub(super) ref_name: String,
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

/// GitHub REST API client for PR operations.
pub(super) struct GithubPrApi {
    client: Client,
    token: String,
    base_url: String,
}

impl GithubPrApi {
    pub(super) fn new(token: &str) -> Self {
        Self {
            client: Client::new(),
            token: token.to_string(),
            base_url: moatless_core::config::api_base(),
        }
    }

    /// Point the client at a caller-supplied REST root so tests can drive the
    /// real request path against a local address.
    #[cfg(test)]
    pub(super) fn with_base_url(token: &str, base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
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

        // Truncate if needed
        let body = if body.len() > MAX_COMMENT_LENGTH {
            &body[..MAX_COMMENT_LENGTH]
        } else {
            body
        };

        let resp = self
            .request(reqwest::Method::POST, &url)
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub post_comment", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub post_comment error");
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub post_comment returned {status}: {text}"
            ))
            .into());
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

    /// Create a pull request. Returns the raw GitHub PR payload so callers
    /// can extract the fields they need (`number`, `html_url`, ...).
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
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub create_pr error");
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub create_pr returned {status}: {text}"
            ))
            .into());
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
            let text = resp.text().await.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub get_repo returned {status}: {text}"
            ))
            .into());
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
    /// Calls `GET /repos/{owner}/{repo}/commits/{sha}/pulls`. Used to map a
    /// main-branch CI run (which carries no PR reference in its webhook) back
    /// to the PR whose merge produced the commit — GitHub associates the merge
    /// (or squash) commit with the merged PR.
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
            let text = resp.text().await.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub get_pulls_for_commit returned {status}: {text}"
            ))
            .into());
        }

        resp.json::<Vec<CommitPr>>().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub get_pulls_for_commit", e).into()
        })
    }

    /// The open pull request whose head branch is `head`, if any.
    ///
    /// `GET /repos/{owner}/{repo}/pulls?head={owner}:{head}&state=open`. GitHub
    /// filters `head` as `{owner}:{branch}`, so a fork's like-named branch is
    /// not mistaken for this repository's. A head branch has at most one open
    /// pull request, so the first match is the answer. Serves three callers: the
    /// stacked action's `opened_existing` path, the create-PR endpoint's 422
    /// recovery, and binding a Task prepared from a pull-request reference.
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
            let text = resp.text().await.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub find_open_pr_for_head returned {status}: {text}"
            ))
            .into());
        }

        let prs: Vec<PrInfo> = resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub find_open_pr_for_head", e)
        })?;
        Ok(prs.into_iter().next())
    }

    /// Reply to a PR review comment (threaded review reply).
    ///
    /// Calls `POST /repos/{owner}/{repo}/pulls/{pr}/comments/{cid}/replies` and
    /// returns the new review comment id.
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

        let body = if body.len() > MAX_COMMENT_LENGTH {
            &body[..MAX_COMMENT_LENGTH]
        } else {
            body
        };

        let resp = self
            .request(reqwest::Method::POST, &url)
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_unreachable("GitHub reply_to_review_comment", e)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub reply_to_review_comment error");
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub reply_to_review_comment returned {status}: {text}"
            ))
            .into());
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
            let text = resp.text().await.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub get_pr returned {status}: {text}"
            ))
            .into());
        }

        resp.json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("GitHub get_pr", e).into())
    }

    /// Fetch PR metadata, polling until GitHub has computed the mergeable
    /// state (or a small retry budget is exhausted).
    ///
    /// GitHub computes mergeability asynchronously: a PR read right after a push
    /// returns `mergeable: null` / `mergeable_state: "unknown"`, and the read
    /// itself is what loops the background job. We retry a few times with a
    /// short backoff so callers get a settled `clean`/`dirty`/… value. If it
    /// never settles within the budget, the last (possibly `unknown`) `PrInfo`
    /// is returned rather than erroring — callers treat `unknown` as
    /// "inconclusive" and simply don't notify.
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
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub create_reaction error");
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub create_reaction returned {status}: {text}"
            ))
            .into());
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
            let text = resp.text().await.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub list_reactions returned {status}: {text}"
            ))
            .into());
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
            let text = resp.text().await.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub delete_reaction returned {status}: {text}"
            ))
            .into());
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
            let text = resp.text().await.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "GitHub user returned {status}: {text}"
            ))
            .into());
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
        let api = GithubPrApi::with_base_url("ghp_test", UNREACHABLE_BASE);

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
        let api = GithubPrApi::with_base_url("ghp_test", base);

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

    /// GitHub answering 403 is a rejection, not a wrapped failure.
    #[tokio::test]
    async fn github_rejection_stays_a_synthesized_message() {
        let base = stub_provider(StatusCode::FORBIDDEN, r#"{"message":"Bad credentials"}"#).await;
        let api = GithubPrApi::with_base_url("ghp_test", base);

        let err = api
            .post_comment("acme", "widgets", 7, "hi")
            .await
            .expect_err("403 must be an error");

        assert!(
            matches!(err, AdapterError::Shared(SharedAdapterError::AdapterApi(_))),
            "got {err:?}"
        );
        assert!(err.source().is_none());
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

        let api = GithubPrApi::with_base_url("ghp_test", format!("http://{address}"));
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

        let api = GithubPrApi::with_base_url("ghp_test", format!("http://{address}"));
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

        let api = GithubPrApi::with_base_url("ghp_test", format!("http://{address}"));
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
