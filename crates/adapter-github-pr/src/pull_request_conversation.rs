//! A pull request's conversation, commits and status read from GitHub's
//! GraphQL API, for the T3 pull request page.

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::adapter::GithubPrAdapter;
use super::api::GithubPrApi;
use super::auth::resolve_github_api;
use super::pull_request_search::{Nodes, PageInfo};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::pull_request_conversation::{
    DiffSide, PullRequestActivity, PullRequestActor, PullRequestCheck, PullRequestCheckStatus,
    PullRequestCommitEntry, PullRequestLabel, PullRequestReaction, PullRequestRemark,
    PullRequestRemarkKind, PullRequestStatus, PullRequestViewerRights, ReviewThread,
    ReviewThreadComment, ReviewThreadCommentPage,
};

/// Pages [`pull_request_activity`] reads before reporting the rest truncated:
/// each connection gives up to a hundred entries a page.
const MAX_ACTIVITY_PAGES: usize = 10;

const ACTOR: &str = "__typename login avatarUrl ... on User { name }";

/// A reaction group's fields; `reactors` is bounded rather than paged, since a
/// hover names a few and counts the rest.
const REACTIONS: &str = "content viewerHasReacted reactors(first: 10) { totalCount nodes { ... on User { login } ... on Bot { login } ... on Organization { login } ... on Mannequin { login } } }";

/// `query` with its `ACTOR` and `REACTIONS` placeholders filled in.
fn expand(query: &str) -> String {
    query
        .replace("ACTOR", ACTOR)
        .replace("REACTIONS", REACTIONS)
}

/// Each connection is switched on only while it has more to give; the first
/// page (`$head`) also reads what is not paged.
const ACTIVITY_QUERY: &str = r#"query($owner: String!, $name: String!, $number: Int!, $head: Boolean!, $withComments: Boolean!, $commentsAfter: String, $withReviews: Boolean!, $reviewsAfter: String, $withThreads: Boolean!, $threadsAfter: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      author @include(if: $head) { ACTOR }
      reactionGroups @include(if: $head) { REACTIONS }
      reviewRequests(first: 50) @include(if: $head) {
        nodes { requestedReviewer { __typename ... on User { login name avatarUrl } ... on Bot { login avatarUrl } ... on Team { combinedSlug name avatarUrl } } }
      }
      latestReviews(first: 50) @include(if: $head) { nodes { author { ACTOR } } }
      commits(last: 100) @include(if: $head) {
        nodes { commit { oid messageHeadline committedDate additions deletions authors(first: 3) { nodes { name avatarUrl user { login } } } } }
      }
      comments(first: 100, after: $commentsAfter) @include(if: $withComments) {
        totalCount
        pageInfo { hasNextPage endCursor }
        nodes { id body createdAt lastEditedAt url viewerCanUpdate reactionGroups { REACTIONS } author { ACTOR } }
      }
      reviews(first: 100, after: $reviewsAfter) @include(if: $withReviews) {
        pageInfo { hasNextPage endCursor }
        nodes { id body state submittedAt lastEditedAt url viewerCanUpdate reactionGroups { REACTIONS } author { ACTOR } }
      }
      reviewThreads(first: 100, after: $threadsAfter) @include(if: $withThreads) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id isResolved isOutdated path line diffSide viewerCanResolve viewerCanUnresolve
          comments(first: 10) {
            totalCount
            pageInfo { hasNextPage endCursor }
            nodes { id body createdAt lastEditedAt url viewerCanUpdate reactionGroups { REACTIONS } author { ACTOR } }
          }
        }
      }
    }
  }
}"#;

/// The thread's pull request is read so a thread id from elsewhere is refused.
const THREAD_COMMENTS_QUERY: &str = r#"query($threadId: ID!, $cursor: String) {
  node(id: $threadId) {
    ... on PullRequestReviewThread {
      pullRequest { number repository { nameWithOwner } }
      comments(first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes { id body createdAt lastEditedAt url viewerCanUpdate reactionGroups { REACTIONS } author { ACTOR } }
      }
    }
  }
}"#;

const STATUS_QUERY: &str = r#"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    mergeCommitAllowed squashMergeAllowed rebaseMergeAllowed autoMergeAllowed viewerPermission
    pullRequest(number: $number) {
      viewerCanUpdate viewerDidAuthor viewerCanUpdateBranch mergeStateStatus
      autoMergeRequest { mergeMethod }
      headRepository { viewerPermission }
      headRef { id }
      labels(first: 100) { nodes { name color } }
      reviewRequests(first: 100) {
        nodes { requestedReviewer { __typename ... on User { login name avatarUrl } ... on Bot { login avatarUrl } ... on Team { combinedSlug name avatarUrl } } }
      }
      commits(last: 1) {
        nodes { commit { statusCheckRollup { contexts(first: 100) { nodes {
          __typename
          ... on StatusContext { context state targetUrl description }
          ... on CheckRun { name status conclusion detailsUrl title }
        } } } } }
      }
    }
  }
}"#;

/// `owner/repo#number`'s conversation: its issue comments, reviews and review
/// threads, the people on its review, and its newest hundred commits.
pub async fn pull_request_activity(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<PullRequestActivity, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    read_activity(&api, owner, repo, number).await
}

async fn read_activity(
    api: &GithubPrApi,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<PullRequestActivity, AdapterError> {
    let query = expand(ACTIVITY_QUERY);
    let mut read = ActivityRead::default();
    let (mut comments_after, mut reviews_after, mut threads_after) = (None, None, None);
    let (mut with_comments, mut with_reviews, mut with_threads) = (true, true, true);
    for page in 0..MAX_ACTIVITY_PAGES {
        if !(with_comments || with_reviews || with_threads) {
            break;
        }
        let pr: ActivityPullRequest = pull_request_data(
            api,
            &query,
            serde_json::json!({
                "owner": owner,
                "name": repo,
                "number": number,
                "head": page == 0,
                "withComments": with_comments,
                "commentsAfter": comments_after,
                "withReviews": with_reviews,
                "reviewsAfter": reviews_after,
                "withThreads": with_threads,
                "threadsAfter": threads_after,
            }),
        )
        .await?;
        comments_after = read.take_comments(pr.comments);
        reviews_after = read.take_reviews(pr.reviews);
        threads_after = read.take_threads(pr.review_threads);
        if page == 0 {
            read.reactions = reactions(pr.reaction_groups);
            read.take_head(pr.author, pr.review_requests, pr.latest_reviews, pr.commits);
        }
        with_comments = with_comments && comments_after.is_some();
        with_reviews = with_reviews && reviews_after.is_some();
        with_threads = with_threads && threads_after.is_some();
    }
    Ok(read.finish(with_comments || with_reviews, with_threads))
}

/// The page of thread `thread_id`'s comments after `cursor`. Refused unless
/// the thread is on `owner/repo#number`.
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is one fact of a single read; a struct for them would have one caller"
)]
pub async fn review_thread_comments(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    number: u64,
    thread_id: &str,
    cursor: Option<&str>,
) -> Result<ReviewThreadCommentPage, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    read_thread_comments(&api, owner, repo, number, thread_id, cursor).await
}

async fn read_thread_comments(
    api: &GithubPrApi,
    owner: &str,
    repo: &str,
    number: u64,
    thread_id: &str,
    cursor: Option<&str>,
) -> Result<ReviewThreadCommentPage, AdapterError> {
    let data = api
        .graphql(
            &expand(THREAD_COMMENTS_QUERY),
            serde_json::json!({ "threadId": thread_id, "cursor": cursor }),
        )
        .await?;
    let data: ThreadNodeData = decode(data)?;
    let thread = data.node.filter(|thread| {
        thread.pull_request.as_ref().is_some_and(|pr| {
            pr.number == number
                && pr
                    .repository
                    .name_with_owner
                    .eq_ignore_ascii_case(&format!("{owner}/{repo}"))
        })
    });
    let Some(comments) = thread.and_then(|thread| thread.comments) else {
        return Err(SharedAdapterError::AdapterApi(format!(
            "Review thread {thread_id} is not on {owner}/{repo}#{number}"
        ))
        .into());
    };
    Ok(ReviewThreadCommentPage {
        comments: comments
            .nodes
            .into_iter()
            .flatten()
            .map(RawComment::into_thread_comment)
            .collect(),
        next_cursor: comments.page_info.next(),
    })
}

/// `owner/repo#number`'s labels, requested reviewers and head commit checks.
pub async fn pull_request_status(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<PullRequestStatus, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    read_status(&api, owner, repo, number).await
}

async fn read_status(
    api: &GithubPrApi,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<PullRequestStatus, AdapterError> {
    let data: StatusData = decode(
        api.graphql(
            STATUS_QUERY,
            serde_json::json!({ "owner": owner, "name": repo, "number": number }),
        )
        .await?,
    )?;
    let (settings, pr) = data
        .repository
        .and_then(|repository| {
            let pr = repository.pull_request?;
            Some((repository.settings, pr))
        })
        .ok_or_else(|| {
            AdapterError::from(SharedAdapterError::AdapterApi(
                "GitHub has no such pull request".to_string(),
            ))
        })?;
    let viewer = Some(viewer_rights(&settings, &pr.viewer));
    Ok(PullRequestStatus {
        viewer,
        labels: pr
            .labels
            .nodes
            .into_iter()
            .flatten()
            .map(|label| PullRequestLabel {
                name: label.name,
                color: label.color,
            })
            .collect(),
        requested_reviewers: requested_reviewers(pr.review_requests),
        checks: pr
            .commits
            .nodes
            .into_iter()
            .flatten()
            .next_back()
            .and_then(|node| node.commit.status_check_rollup)
            .map_or_default(|rollup| {
                rollup
                    .contexts
                    .nodes
                    .into_iter()
                    .flatten()
                    .filter_map(RawCheck::into_check)
                    .collect()
            }),
    })
}

/// `query`'s `repository.pullRequest`, failing when GitHub has no such pull
/// request the token can read.
pub(super) async fn pull_request_data<T: DeserializeOwned>(
    api: &GithubPrApi,
    query: &str,
    variables: serde_json::Value,
) -> Result<T, AdapterError> {
    let data: RepositoryData<T> = decode(api.graphql(query, variables).await?)?;
    data.repository
        .and_then(|repository| repository.pull_request)
        .ok_or_else(|| {
            SharedAdapterError::AdapterApi("GitHub has no such pull request".to_string()).into()
        })
}

fn decode<T: DeserializeOwned>(data: serde_json::Value) -> Result<T, AdapterError> {
    serde_json::from_value(data).map_err(|e| {
        AdapterError::from(SharedAdapterError::AdapterApi(format!(
            "GitHub pull request read: unexpected response: {e}"
        )))
    })
}

/// What [`pull_request_activity`] has read so far.
#[derive(Default)]
struct ActivityRead {
    author: Option<PullRequestActor>,
    reviewers: Vec<PullRequestActor>,
    commits: Vec<PullRequestCommitEntry>,
    remarks: Vec<PullRequestRemark>,
    issue_comment_total: u64,
    issue_comments_read: u64,
    /// Comments the read threads hold beyond their first page.
    unread_thread_comments: u64,
    threads: Vec<ReviewThread>,
    reactions: Vec<PullRequestReaction>,
}

impl ActivityRead {
    fn take_head(
        &mut self,
        author: Option<RawActor>,
        review_requests: Option<Nodes<RawReviewRequest>>,
        latest_reviews: Option<Nodes<RawLatestReview>>,
        commits: Option<Nodes<RawCommitNode>>,
    ) {
        self.author = author.map(RawActor::into_actor);
        let mut reviewers = requested_reviewers(review_requests.unwrap_or_default());
        for actor in latest_reviews
            .unwrap_or_default()
            .nodes
            .into_iter()
            .flatten()
            .filter_map(|review| review.author)
            .map(RawActor::into_actor)
        {
            if !reviewers.iter().any(|known| known.login == actor.login) {
                reviewers.push(actor);
            }
        }
        self.reviewers = reviewers;
        self.commits = commits
            .unwrap_or_default()
            .nodes
            .into_iter()
            .flatten()
            .map(|node| node.commit.into_entry())
            .collect();
    }

    fn take_comments(&mut self, comments: Option<RawCommentConnection>) -> Option<String> {
        let comments = comments?;
        // Every page repeats the same total.
        self.issue_comment_total = comments.total_count;
        for comment in comments.nodes.into_iter().flatten() {
            self.issue_comments_read += 1;
            self.remarks
                .push(comment.into_remark(PullRequestRemarkKind::IssueComment, None));
        }
        comments.page_info.next()
    }

    fn take_reviews(&mut self, reviews: Option<RawReviewConnection>) -> Option<String> {
        let reviews = reviews?;
        self.remarks.extend(
            reviews
                .nodes
                .into_iter()
                .flatten()
                .filter_map(RawReview::into_remark),
        );
        reviews.page_info.next()
    }

    fn take_threads(&mut self, threads: Option<RawThreadConnection>) -> Option<String> {
        let threads = threads?;
        for thread in threads.nodes.into_iter().flatten() {
            let thread = thread.into_thread();
            self.unread_thread_comments += thread
                .comment_count
                .saturating_sub(u64::try_from(thread.comments.len()).unwrap_or(u64::MAX));
            self.remarks
                .extend(thread.comments.iter().map(|comment| PullRequestRemark {
                    id: comment.id.clone(),
                    kind: PullRequestRemarkKind::ReviewComment,
                    author: comment.author.clone(),
                    body: comment.body.clone(),
                    created_at: comment.created_at.clone(),
                    edited_at: comment.edited_at.clone(),
                    url: comment.url.clone(),
                    path: Some(thread.path.clone()),
                    review_state: None,
                    can_edit: comment.can_edit,
                    reactions: comment.reactions.clone(),
                }));
            self.threads.push(thread);
        }
        threads.page_info.next()
    }

    fn finish(mut self, remarks_cut: bool, threads_cut: bool) -> PullRequestActivity {
        self.remarks
            .sort_by(|left, right| left.created_at.cmp(&right.created_at));
        let unread = self
            .issue_comment_total
            .saturating_sub(self.issue_comments_read)
            + self.unread_thread_comments;
        let read = u64::try_from(self.remarks.len()).unwrap_or(u64::MAX);
        PullRequestActivity {
            author: self.author,
            reviewers: self.reviewers,
            remark_count: read + unread,
            remarks_truncated: remarks_cut || threads_cut || unread > 0,
            remarks: self.remarks,
            review_threads: self.threads,
            review_threads_truncated: threads_cut,
            commits: self.commits,
            reactions: self.reactions,
        }
    }
}

fn requested_reviewers(requests: Nodes<RawReviewRequest>) -> Vec<PullRequestActor> {
    requests
        .nodes
        .into_iter()
        .flatten()
        .filter_map(|request| request.requested_reviewer?.into_actor())
        .collect()
}

#[derive(Deserialize)]
struct RepositoryData<T> {
    repository: Option<PullRequestData<T>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestData<T> {
    pull_request: Option<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivityPullRequest {
    author: Option<RawActor>,
    reaction_groups: Option<Vec<RawReactionGroup>>,
    review_requests: Option<Nodes<RawReviewRequest>>,
    latest_reviews: Option<Nodes<RawLatestReview>>,
    commits: Option<Nodes<RawCommitNode>>,
    comments: Option<RawCommentConnection>,
    reviews: Option<RawReviewConnection>,
    review_threads: Option<RawThreadConnection>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawActor {
    #[serde(rename = "__typename")]
    typename: Option<String>,
    login: String,
    avatar_url: Option<String>,
    name: Option<String>,
}

impl RawActor {
    fn into_actor(self) -> PullRequestActor {
        PullRequestActor {
            is_bot: self.typename.as_deref() == Some("Bot"),
            login: self.login,
            name: self.name.filter(|name| !name.trim().is_empty()),
            avatar_url: self.avatar_url,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReviewRequest {
    requested_reviewer: Option<RawRequestedReviewer>,
}

/// A user, bot or team; a team goes by `org/slug`, which no login contains.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRequestedReviewer {
    #[serde(rename = "__typename")]
    typename: Option<String>,
    login: Option<String>,
    combined_slug: Option<String>,
    name: Option<String>,
    avatar_url: Option<String>,
}

impl RawRequestedReviewer {
    fn into_actor(self) -> Option<PullRequestActor> {
        Some(PullRequestActor {
            is_bot: self.typename.as_deref() == Some("Bot"),
            login: self.login.or(self.combined_slug)?,
            name: self.name.filter(|name| !name.trim().is_empty()),
            avatar_url: self.avatar_url,
        })
    }
}

#[derive(Deserialize)]
struct RawLatestReview {
    author: Option<RawActor>,
}

#[derive(Deserialize)]
struct RawCommitNode {
    commit: RawCommit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCommit {
    oid: String,
    message_headline: String,
    committed_date: String,
    additions: Option<u64>,
    deletions: Option<u64>,
    #[serde(default)]
    authors: Nodes<RawCommitAuthor>,
}

impl RawCommit {
    fn into_entry(self) -> PullRequestCommitEntry {
        PullRequestCommitEntry {
            oid: self.oid,
            message_headline: self.message_headline,
            committed_date: self.committed_date,
            additions: self.additions,
            deletions: self.deletions,
            authors: self
                .authors
                .nodes
                .into_iter()
                .flatten()
                .filter_map(RawCommitAuthor::into_actor)
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCommitAuthor {
    name: Option<String>,
    avatar_url: Option<String>,
    user: Option<RawLogin>,
}

impl RawCommitAuthor {
    /// Named by the git author name where no GitHub user is linked.
    fn into_actor(self) -> Option<PullRequestActor> {
        let name = self
            .name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        let login = self.user.map(|user| user.login).or_else(|| name.clone())?;
        Some(PullRequestActor {
            login,
            name,
            avatar_url: self.avatar_url,
            is_bot: false,
        })
    }
}

#[derive(Deserialize)]
struct RawLogin {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCommentConnection {
    #[serde(default)]
    total_count: u64,
    page_info: PageInfo,
    #[serde(default)]
    nodes: Vec<Option<RawComment>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawComment {
    id: String,
    #[serde(default)]
    body: String,
    created_at: String,
    last_edited_at: Option<String>,
    url: Option<String>,
    author: Option<RawActor>,
    viewer_can_update: Option<bool>,
    reaction_groups: Option<Vec<RawReactionGroup>>,
}

impl RawComment {
    fn into_remark(self, kind: PullRequestRemarkKind, path: Option<String>) -> PullRequestRemark {
        PullRequestRemark {
            id: self.id,
            kind,
            author: self.author.map(RawActor::into_actor),
            body: self.body,
            created_at: self.created_at,
            edited_at: self.last_edited_at,
            url: self.url,
            path,
            review_state: None,
            can_edit: self.viewer_can_update,
            reactions: reactions(self.reaction_groups),
        }
    }

    fn into_thread_comment(self) -> ReviewThreadComment {
        ReviewThreadComment {
            id: self.id,
            author: self.author.map(RawActor::into_actor),
            body: self.body,
            created_at: self.created_at,
            edited_at: self.last_edited_at,
            url: self.url,
            can_edit: self.viewer_can_update,
            reactions: reactions(self.reaction_groups),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReviewConnection {
    page_info: PageInfo,
    #[serde(default)]
    nodes: Vec<Option<RawReview>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReview {
    id: String,
    #[serde(default)]
    body: String,
    state: String,
    submitted_at: Option<String>,
    last_edited_at: Option<String>,
    url: Option<String>,
    author: Option<RawActor>,
    viewer_can_update: Option<bool>,
    reaction_groups: Option<Vec<RawReactionGroup>>,
}

impl RawReview {
    /// `None` for a pending review, and for a bodiless `COMMENTED` one: GitHub
    /// opens that as the container of line comments, which are read from the
    /// threads, so keeping it would show a name with nothing under it.
    fn into_remark(self) -> Option<PullRequestRemark> {
        let created_at = self.submitted_at?;
        if self.body.trim().is_empty() && self.state == "COMMENTED" {
            return None;
        }
        Some(PullRequestRemark {
            id: self.id,
            kind: PullRequestRemarkKind::Review,
            author: self.author.map(RawActor::into_actor),
            body: self.body,
            created_at,
            edited_at: self.last_edited_at,
            url: self.url,
            path: None,
            review_state: Some(self.state),
            can_edit: self.viewer_can_update,
            reactions: reactions(self.reaction_groups),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThreadConnection {
    page_info: PageInfo,
    #[serde(default)]
    nodes: Vec<Option<RawThread>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThread {
    id: String,
    is_resolved: bool,
    is_outdated: bool,
    path: String,
    line: Option<u64>,
    diff_side: Option<String>,
    comments: RawCommentConnection,
    viewer_can_resolve: Option<bool>,
    viewer_can_unresolve: Option<bool>,
}

impl RawThread {
    fn into_thread(self) -> ReviewThread {
        let comments: Vec<ReviewThreadComment> = self
            .comments
            .nodes
            .into_iter()
            .flatten()
            .map(RawComment::into_thread_comment)
            .collect();
        ReviewThread {
            id: self.id,
            path: self.path,
            line: self.line.filter(|line| *line > 0),
            side: match self.diff_side.as_deref() {
                Some("LEFT") => DiffSide::Left,
                _ => DiffSide::Right,
            },
            is_resolved: self.is_resolved,
            is_outdated: self.is_outdated,
            comment_count: self
                .comments
                .total_count
                .max(u64::try_from(comments.len()).unwrap_or(u64::MAX)),
            next_comments_cursor: self.comments.page_info.next(),
            comments,
            // Whichever way the thread would turn now.
            can_resolve: if self.is_resolved {
                self.viewer_can_unresolve
            } else {
                self.viewer_can_resolve
            },
        }
    }
}

/// T3's names of GitHub's `ReactionContent`; others are left out.
const REACTION_NAMES: [(&str, &str); 8] = [
    ("THUMBS_UP", "thumbs-up"),
    ("THUMBS_DOWN", "thumbs-down"),
    ("LAUGH", "laugh"),
    ("HOORAY", "hooray"),
    ("CONFUSED", "confused"),
    ("HEART", "heart"),
    ("ROCKET", "rocket"),
    ("EYES", "eyes"),
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReactionGroup {
    content: String,
    #[serde(default)]
    viewer_has_reacted: bool,
    reactors: RawReactors,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReactors {
    #[serde(default)]
    total_count: u64,
    #[serde(default)]
    nodes: Vec<Option<RawMaybeLogin>>,
}

/// A reactor GitHub no longer names reads as `{}`.
#[derive(Deserialize)]
struct RawMaybeLogin {
    login: Option<String>,
}

/// The groups anyone has reacted in, by T3's name.
fn reactions(groups: Option<Vec<RawReactionGroup>>) -> Vec<PullRequestReaction> {
    groups
        .unwrap_or_default()
        .into_iter()
        .filter(|group| group.reactors.total_count > 0)
        .filter_map(|group| {
            let (_, name) = REACTION_NAMES
                .iter()
                .find(|(github, _)| *github == group.content)?;
            Some(PullRequestReaction {
                content: (*name).to_string(),
                count: group.reactors.total_count,
                actors: group
                    .reactors
                    .nodes
                    .into_iter()
                    .flatten()
                    .filter_map(|reactor| reactor.login)
                    .collect(),
                viewer_has_reacted: group.viewer_has_reacted,
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct ThreadNodeData {
    node: Option<RawThreadNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThreadNode {
    pull_request: Option<RawThreadPullRequest>,
    comments: Option<RawCommentConnection>,
}

#[derive(Deserialize)]
struct RawThreadPullRequest {
    number: u64,
    repository: RawRepositoryName,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepositoryName {
    name_with_owner: String,
}

#[derive(Deserialize)]
struct StatusData {
    repository: Option<StatusRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusRepository {
    #[serde(flatten)]
    settings: RepositorySettings,
    pull_request: Option<StatusPullRequest>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositorySettings {
    #[serde(default)]
    merge_commit_allowed: bool,
    #[serde(default)]
    squash_merge_allowed: bool,
    #[serde(default)]
    rebase_merge_allowed: bool,
    #[serde(default)]
    auto_merge_allowed: bool,
    viewer_permission: Option<String>,
}

/// What the status read says the viewer may do to the pull request itself.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestViewer {
    #[serde(default)]
    viewer_can_update: bool,
    #[serde(default)]
    viewer_did_author: bool,
    #[serde(default)]
    viewer_can_update_branch: bool,
    merge_state_status: Option<String>,
    auto_merge_request: Option<RawAutoMerge>,
    head_repository: Option<RawPermission>,
    head_ref: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAutoMerge {
    merge_method: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPermission {
    viewer_permission: Option<String>,
}

/// GitHub answers `null` only to an App installation, whose rights no API
/// reports: it is granted, and GitHub refuses what the App was not given.
fn permission_at_least_write(permission: Option<&str>) -> bool {
    permission.is_none_or(|permission| matches!(permission, "ADMIN" | "MAINTAIN" | "WRITE"))
}

fn viewer_rights(settings: &RepositorySettings, pr: &PullRequestViewer) -> PullRequestViewerRights {
    let permission = settings.viewer_permission.as_deref();
    let can_write = permission_at_least_write(permission);
    PullRequestViewerRights {
        can_write,
        can_triage: can_write || permission == Some("TRIAGE"),
        can_delete_branch: pr.head_ref.is_some()
            && pr
                .head_repository
                .as_ref()
                .is_some_and(|head| permission_at_least_write(head.viewer_permission.as_deref())),
        can_update: pr.viewer_can_update,
        did_author: pr.viewer_did_author,
        can_update_branch: pr.viewer_can_update_branch,
        // The states in which GitHub merges at once rather than arming
        // auto-merge.
        can_merge_now: matches!(
            pr.merge_state_status.as_deref(),
            Some("CLEAN" | "HAS_HOOKS" | "UNSTABLE")
        ),
        merge_commit_allowed: settings.merge_commit_allowed,
        squash_merge_allowed: settings.squash_merge_allowed,
        rebase_merge_allowed: settings.rebase_merge_allowed,
        auto_merge_allowed: settings.auto_merge_allowed,
        auto_merge_method: pr
            .auto_merge_request
            .as_ref()
            .and_then(|request| request.merge_method.as_deref())
            .map(str::to_ascii_lowercase),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusPullRequest {
    #[serde(flatten)]
    viewer: PullRequestViewer,
    #[serde(default)]
    labels: Nodes<RawLabel>,
    #[serde(default)]
    review_requests: Nodes<RawReviewRequest>,
    #[serde(default)]
    commits: Nodes<RawStatusCommitNode>,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
    color: Option<String>,
}

#[derive(Deserialize)]
struct RawStatusCommitNode {
    commit: RawStatusCommit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawStatusCommit {
    status_check_rollup: Option<RawRollup>,
}

#[derive(Deserialize)]
struct RawRollup {
    contexts: Nodes<RawCheck>,
}

/// A `StatusContext` (`context`, `state`) or a `CheckRun` (`name`, `status`,
/// `conclusion`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCheck {
    context: Option<String>,
    state: Option<String>,
    target_url: Option<String>,
    description: Option<String>,
    name: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    details_url: Option<String>,
    title: Option<String>,
}

impl RawCheck {
    fn into_check(self) -> Option<PullRequestCheck> {
        let name = self
            .name
            .or(self.context)
            .filter(|name| !name.trim().is_empty())?;
        Some(PullRequestCheck {
            status: check_status(
                self.status.as_deref(),
                self.conclusion.as_deref().or(self.state.as_deref()),
            ),
            name,
            description: self.title.or(self.description),
            url: self.details_url.or(self.target_url),
            required: None,
        })
    }
}

/// A check run reports `status` plus a `conclusion` once completed; a commit
/// status reports only `state`, passed here as `outcome`.
fn check_status(status: Option<&str>, outcome: Option<&str>) -> PullRequestCheckStatus {
    if status.is_some_and(|status| !status.is_empty() && status != "COMPLETED") {
        return PullRequestCheckStatus::Pending;
    }
    match outcome {
        Some("SUCCESS") => PullRequestCheckStatus::Success,
        Some("ACTION_REQUIRED") => PullRequestCheckStatus::ActionRequired,
        Some("FAILURE" | "ERROR" | "TIMED_OUT" | "STARTUP_FAILURE") => {
            PullRequestCheckStatus::Failure
        }
        Some("CANCELLED") => PullRequestCheckStatus::Cancelled,
        Some("SKIPPED") => PullRequestCheckStatus::Skipped,
        Some("PENDING" | "EXPECTED") => PullRequestCheckStatus::Pending,
        _ => PullRequestCheckStatus::Neutral,
    }
}

#[cfg(test)]
#[path = "pull_request_conversation_tests.rs"]
mod tests;
