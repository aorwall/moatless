//! A repository's pull requests as GitHub's search finds them, for the T3
//! pull request list.

use serde::Deserialize;

use super::adapter::GithubPrAdapter;
use super::auth::resolve_github_api;
use super::status::checks_state;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};

/// Most rows one GraphQL search page returns.
pub const MAX_PAGE_SIZE: u32 = 100;

/// What to list. Every field narrows the search; the defaults list every
/// pull request in the repository.
#[derive(Debug, Default, Clone)]
pub struct PullRequestSearch {
    /// `open`, `closed` (closed unmerged) or `merged`; anything else is all.
    pub state: String,
    pub involvement: Option<Involvement>,
    pub text: Option<String>,
    /// `only` or `hide`.
    pub draft: Option<String>,
    /// `approved`, `changes-requested`, `review-required` or `none`.
    pub review: Option<String>,
    /// `passing` or `failing`.
    pub checks: Option<String>,
    /// Each group matches any of its labels; every group must match.
    pub labels: Vec<Vec<String>>,
    pub excluded_labels: Vec<String>,
    pub author: Option<String>,
}

/// Pull requests one GitHub user takes part in, named by login rather than
/// `@me`: the token may belong to a Workspace's credential, not to the viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Involvement {
    Authored(String),
    ReviewRequested(String),
}

/// One pull request in a search page.
#[derive(Debug, Clone, PartialEq)]
pub struct ListedPullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author_login: Option<String>,
    pub author_avatar_url: Option<String>,
    pub author_is_bot: bool,
    pub head_ref: String,
    pub base_ref: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    pub is_draft: bool,
    /// `mergeable`, `conflicting` or `unknown`.
    pub mergeability: String,
    pub additions: u64,
    pub deletions: u64,
    pub created_at: String,
    pub updated_at: String,
    /// Name and hex colour without `#`.
    pub labels: Vec<(String, Option<String>)>,
    /// `approved`, `changes-requested` or `review-required`.
    pub review_decision: Option<String>,
    /// `passing`, `failing` or `pending`.
    pub checks_state: Option<String>,
    /// Logins of the users whose review is requested.
    pub requested_reviewers: Vec<String>,
}

/// One page of [`search_pull_requests`], newest update first.
#[derive(Debug, Clone, PartialEq)]
pub struct PullRequestSearchPage {
    pub items: Vec<ListedPullRequest>,
    /// Hand back as `after` for the next page; `None` on the last one.
    pub next_cursor: Option<String>,
}

const QUERY: &str = r#"query($q: String!, $first: Int!, $after: String) {
  search(query: $q, type: ISSUE, first: $first, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes {
      ... on PullRequest {
        number title url state isDraft mergeable additions deletions
        createdAt updatedAt headRefName baseRefName reviewDecision
        author { __typename login avatarUrl }
        labels(first: 20) { nodes { name color } }
        reviewRequests(first: 20) {
          nodes { requestedReviewer { ... on User { login } } }
        }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
      }
    }
  }
}"#;

/// Up to `first` (at most [`MAX_PAGE_SIZE`]) of `owner/repo`'s pull requests
/// matching `search`, read with the caller's GitHub credentials.
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is one fact of a single search; a struct for them would have one caller"
)]
pub async fn search_pull_requests(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    search: &PullRequestSearch,
    first: u32,
    after: Option<&str>,
) -> Result<PullRequestSearchPage, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    let data = api
        .graphql(
            QUERY,
            serde_json::json!({
                "q": search_query(owner, repo, search),
                "first": first.clamp(1, MAX_PAGE_SIZE),
                "after": after,
            }),
        )
        .await?;
    let data: SearchData = serde_json::from_value(data).map_err(|e| {
        AdapterError::from(SharedAdapterError::AdapterApi(format!(
            "GitHub pull request search: unexpected response: {e}"
        )))
    })?;
    Ok(PullRequestSearchPage {
        next_cursor: data.search.page_info.next(),
        items: data
            .search
            .nodes
            .into_iter()
            .flatten()
            .filter_map(|node| node.number.is_some().then(|| node.into_listed()))
            .collect(),
    })
}

/// GitHub's search syntax for `search` in `owner/repo`.
fn search_query(owner: &str, repo: &str, search: &PullRequestSearch) -> String {
    let mut terms = vec![format!("repo:{owner}/{repo}"), "is:pr".to_string()];
    match search.state.as_str() {
        "open" => terms.push("is:open".into()),
        "closed" => terms.extend(["is:closed".into(), "is:unmerged".into()]),
        "merged" => terms.push("is:merged".into()),
        _ => {}
    }
    match &search.involvement {
        Some(Involvement::Authored(login)) => terms.push(format!("author:{}", quoted(login))),
        Some(Involvement::ReviewRequested(login)) => {
            terms.push(format!("review-requested:{}", quoted(login)));
        }
        None => {}
    }
    match search.draft.as_deref() {
        Some("only") => terms.push("draft:true".into()),
        Some("hide") => terms.push("draft:false".into()),
        _ => {}
    }
    match search.review.as_deref() {
        Some("approved") => terms.push("review:approved".into()),
        Some("changes-requested") => terms.push("review:changes_requested".into()),
        Some("review-required") => terms.push("review:required".into()),
        Some("none") => terms.push("review:none".into()),
        _ => {}
    }
    match search.checks.as_deref() {
        Some("passing") => terms.push("status:success".into()),
        Some("failing") => terms.push("status:failure".into()),
        _ => {}
    }
    for group in &search.labels {
        let names: Vec<String> = group.iter().map(|name| quoted(name)).collect();
        if !names.is_empty() {
            terms.push(format!("label:{}", names.join(",")));
        }
    }
    for name in &search.excluded_labels {
        terms.push(format!("-label:{}", quoted(name)));
    }
    if let Some(author) = search.author.as_deref().filter(|a| !a.trim().is_empty()) {
        terms.push(format!("author:{}", quoted(author.trim())));
    }
    // One quoted term per word: the words match anywhere, and none of them
    // can add a qualifier such as `repo:` that widens the search.
    if let Some(text) = search.text.as_deref() {
        terms.extend(
            text.split_whitespace()
                .map(quoted)
                .filter(|term| term != "\"\""),
        );
    }
    terms.push("sort:updated-desc".into());
    terms.join(" ")
}

/// `value` as one search term: a quote inside it would end the term early.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('"', ""))
}

#[derive(Deserialize)]
struct SearchData {
    search: SearchConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchConnection {
    page_info: PageInfo,
    /// A hit the token cannot resolve is `null`.
    #[serde(default)]
    nodes: Vec<Option<PullRequestNode>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PageInfo {
    pub(super) has_next_page: bool,
    pub(super) end_cursor: Option<String>,
}

impl PageInfo {
    /// The cursor of the next page, `None` on the last one.
    pub(super) fn next(self) -> Option<String> {
        self.has_next_page.then_some(self.end_cursor).flatten()
    }
}

#[derive(Deserialize)]
#[serde(bound = "T: Deserialize<'de>")]
pub(super) struct Nodes<T> {
    #[serde(default)]
    pub(super) nodes: Vec<Option<T>>,
}

impl<T> Default for Nodes<T> {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

/// An issue search hit; the fields are absent on an issue that is not a pull
/// request.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct PullRequestNode {
    number: Option<u64>,
    title: String,
    url: String,
    state: String,
    is_draft: bool,
    mergeable: Option<String>,
    additions: u64,
    deletions: u64,
    created_at: String,
    updated_at: String,
    head_ref_name: String,
    base_ref_name: String,
    review_decision: Option<String>,
    author: Option<Author>,
    labels: Nodes<Label>,
    review_requests: Nodes<ReviewRequest>,
    commits: Nodes<CommitNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Author {
    #[serde(rename = "__typename")]
    typename: Option<String>,
    login: String,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
    color: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewRequest {
    requested_reviewer: Option<RequestedReviewer>,
}

#[derive(Deserialize)]
struct RequestedReviewer {
    login: Option<String>,
}

#[derive(Deserialize)]
struct CommitNode {
    commit: Commit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Commit {
    status_check_rollup: Option<Rollup>,
}

#[derive(Deserialize)]
struct Rollup {
    state: String,
}

impl PullRequestNode {
    fn into_listed(self) -> ListedPullRequest {
        let checks_state = self
            .commits
            .nodes
            .into_iter()
            .flatten()
            .next_back()
            .and_then(|node| node.commit.status_check_rollup)
            .and_then(|rollup| checks_state(&rollup.state))
            .map(str::to_string);
        ListedPullRequest {
            number: self.number.unwrap_or_default(),
            title: self.title,
            url: self.url,
            author_is_bot: self
                .author
                .as_ref()
                .is_some_and(|author| author.typename.as_deref() == Some("Bot")),
            author_avatar_url: self
                .author
                .as_ref()
                .and_then(|author| author.avatar_url.clone()),
            author_login: self.author.map(|author| author.login),
            head_ref: self.head_ref_name,
            base_ref: self.base_ref_name,
            state: match self.state.as_str() {
                "MERGED" => "merged",
                "CLOSED" => "closed",
                _ => "open",
            }
            .to_string(),
            is_draft: self.is_draft,
            mergeability: match self.mergeable.as_deref() {
                Some("MERGEABLE") => "mergeable",
                Some("CONFLICTING") => "conflicting",
                _ => "unknown",
            }
            .to_string(),
            additions: self.additions,
            deletions: self.deletions,
            created_at: self.created_at,
            updated_at: self.updated_at,
            labels: self
                .labels
                .nodes
                .into_iter()
                .flatten()
                .map(|label| (label.name, label.color))
                .collect(),
            review_decision: self
                .review_decision
                .as_deref()
                .and_then(|decision| match decision {
                    "APPROVED" => Some("approved".to_string()),
                    "CHANGES_REQUESTED" => Some("changes-requested".to_string()),
                    "REVIEW_REQUIRED" => Some("review-required".to_string()),
                    _ => None,
                }),
            checks_state,
            requested_reviewers: self
                .review_requests
                .nodes
                .into_iter()
                .flatten()
                .filter_map(|request| request.requested_reviewer?.login)
                .collect(),
        }
    }
}

#[cfg(test)]
#[path = "pull_request_search_tests.rs"]
mod tests;
