//! Writes to a pull request from the T3 pull request page, and the candidate
//! lists its menus offer.
//!
//! Every call acts as the viewer through their own GitHub credential, per
//! [`resolve_github_api`], and never as the App: GitHub then authorizes and
//! shows the person who pressed the button, and a viewer who has connected no
//! credential is told to connect one rather than borrowing the App's rights.

use serde::Deserialize;

use super::adapter::GithubPrAdapter;
use super::api::GithubPrApi;
use super::auth::resolve_github_api;
use super::pull_request_conversation::pull_request_data;
use super::pull_request_search::Nodes;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::pull_request_conversation::{
    Candidates, DiffSide, MergeMethod, PullRequestAction, PullRequestActor,
    PullRequestLabelCandidate, PullRequestReviewerCandidate, PullRequestWrite, ReviewCommentDraft,
    ReviewPosition, ReviewVerdict,
};

/// The reaction names T3 sends, each GitHub's `ReactionContent` spelled in
/// kebab case.
const REACTIONS: [&str; 8] = [
    "thumbs-up",
    "thumbs-down",
    "laugh",
    "hooray",
    "confused",
    "heart",
    "rocket",
    "eyes",
];

/// Most candidates a menu lists; GitHub's largest page.
const CANDIDATE_PAGE: u32 = 100;

const NODE_IDS_QUERY: &str = r#"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) { id headRef { id } }
  }
}"#;

const LABEL_CANDIDATES_QUERY: &str = r#"query($owner: String!, $name: String!, $number: Int!, $first: Int!) {
  repository(owner: $owner, name: $name) {
    labels(first: $first) { totalCount nodes { name color description } }
    pullRequest(number: $number) { labels(first: 100) { nodes { name } } }
  }
}"#;

const REVIEWER_CANDIDATES_QUERY: &str = r#"query($owner: String!, $name: String!, $number: Int!, $first: Int!, $query: String) {
  repository(owner: $owner, name: $name) {
    assignableUsers(first: $first, query: $query) { totalCount nodes { login name avatarUrl } }
    pullRequest(number: $number) {
      author { login }
      reviewRequests(first: 100) { nodes { requestedReviewer { __typename ... on User { login } } } }
    }
  }
}"#;

/// Make `write` to `owner/repo#number` as `user_id`.
pub async fn write_pull_request(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    number: u64,
    write: &PullRequestWrite,
) -> Result<(), AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    let issue = format!("repos/{owner}/{repo}/issues/{number}");
    let pull = format!("repos/{owner}/{repo}/pulls/{number}");
    match write {
        PullRequestWrite::Comment { body } => {
            let ids = node_ids(&api, owner, repo, number).await?;
            mutate(
                &api,
                "addComment",
                "mutation($subjectId: ID!, $body: String!) { addComment(input: { subjectId: $subjectId, body: $body }) { clientMutationId } }",
                serde_json::json!({ "subjectId": ids.id, "body": body }),
            )
            .await
        }
        PullRequestWrite::ReplyToThread { thread_id, body } => {
            mutate(
                &api,
                "addPullRequestReviewThreadReply",
                "mutation($threadId: ID!, $body: String!) { addPullRequestReviewThreadReply(input: { pullRequestReviewThreadId: $threadId, body: $body }) { comment { id } } }",
                serde_json::json!({ "threadId": thread_id, "body": body }),
            )
            .await
        }
        PullRequestWrite::SetThreadResolution {
            thread_id,
            resolved,
        } => {
            let field = if *resolved {
                "resolveReviewThread"
            } else {
                "unresolveReviewThread"
            };
            mutate(
                &api,
                field,
                &format!(
                    "mutation($threadId: ID!) {{ {field}(input: {{ threadId: $threadId }}) {{ thread {{ isResolved }} }} }}"
                ),
                serde_json::json!({ "threadId": thread_id }),
            )
            .await
        }
        PullRequestWrite::SubmitReview {
            verdict,
            body,
            comments,
        } => {
            api.rest_write(
                "GitHub submit_review",
                reqwest::Method::POST,
                &format!("{pull}/reviews"),
                Some(review_submission(*verdict, body, comments)),
            )
            .await
        }
        PullRequestWrite::SetReaction {
            subject_id,
            content,
            reacted,
        } => {
            let content = reaction_content(content)?;
            let subject_id = match subject_id {
                Some(id) => id.clone(),
                None => node_ids(&api, owner, repo, number).await?.id,
            };
            let field = if *reacted {
                "addReaction"
            } else {
                "removeReaction"
            };
            mutate(
                &api,
                field,
                &format!(
                    "mutation($subjectId: ID!, $content: ReactionContent!) {{ {field}(input: {{ subjectId: $subjectId, content: $content }}) {{ reaction {{ content }} }} }}"
                ),
                serde_json::json!({ "subjectId": subject_id, "content": content }),
            )
            .await
        }
        PullRequestWrite::Update {
            title,
            body,
            maintainer_can_modify,
        } => {
            let ids = node_ids(&api, owner, repo, number).await?;
            // A field left out of the input is left as it was.
            let mut input = serde_json::json!({ "pullRequestId": ids.id });
            if let Some(title) = title {
                input["title"] = serde_json::json!(title);
            }
            if let Some(body) = body {
                input["body"] = serde_json::json!(body);
            }
            if let Some(modify) = maintainer_can_modify {
                input["maintainerCanModify"] = serde_json::json!(modify);
            }
            mutate(
                &api,
                "updatePullRequest",
                "mutation($input: UpdatePullRequestInput!) { updatePullRequest(input: $input) { pullRequest { id } } }",
                serde_json::json!({ "input": input }),
            )
            .await
        }
        PullRequestWrite::UpdateComment {
            comment_id,
            review_comment,
            body,
        } => {
            let (field, query) = if *review_comment {
                (
                    "updatePullRequestReviewComment",
                    "mutation($id: ID!, $body: String!) { updatePullRequestReviewComment(input: { pullRequestReviewCommentId: $id, body: $body }) { pullRequestReviewComment { id } } }",
                )
            } else {
                (
                    "updateIssueComment",
                    "mutation($id: ID!, $body: String!) { updateIssueComment(input: { id: $id, body: $body }) { issueComment { id } } }",
                )
            };
            mutate(
                &api,
                field,
                query,
                serde_json::json!({ "id": comment_id, "body": body }),
            )
            .await
        }
        PullRequestWrite::SetLabels { labels, applied } => {
            if *applied {
                return api
                    .rest_write(
                        "GitHub add_labels",
                        reqwest::Method::POST,
                        &format!("{issue}/labels"),
                        Some(serde_json::json!({ "labels": labels })),
                    )
                    .await;
            }
            for label in labels {
                api.rest_remove(
                    "GitHub remove_label",
                    &format!("{issue}/labels/{}", path_segment(label)),
                )
                .await?;
            }
            Ok(())
        }
        PullRequestWrite::RequestReviewers {
            users,
            teams,
            requested,
        } => {
            let method = if *requested {
                reqwest::Method::POST
            } else {
                reqwest::Method::DELETE
            };
            api.rest_write(
                "GitHub requested_reviewers",
                method,
                &format!("{pull}/requested_reviewers"),
                Some(serde_json::json!({ "reviewers": users, "team_reviewers": teams })),
            )
            .await
        }
        PullRequestWrite::Action(action) => run_action(&api, owner, repo, number, action).await,
    }
}

async fn run_action(
    api: &GithubPrApi,
    owner: &str,
    repo: &str,
    number: u64,
    action: &PullRequestAction,
) -> Result<(), AdapterError> {
    let ids = node_ids(api, owner, repo, number).await?;
    let by_id = |field: &'static str| {
        (
            field,
            format!(
                "mutation($id: ID!) {{ {field}(input: {{ pullRequestId: $id }}) {{ pullRequest {{ id }} }} }}"
            ),
            serde_json::json!({ "id": ids.id }),
        )
    };
    let (field, query, variables) = match action {
        PullRequestAction::Merge(method) => (
            "mergePullRequest",
            "mutation($id: ID!, $method: PullRequestMergeMethod!) { mergePullRequest(input: { pullRequestId: $id, mergeMethod: $method }) { pullRequest { id } } }".to_string(),
            serde_json::json!({ "id": ids.id, "method": merge_method(*method) }),
        ),
        PullRequestAction::EnableAutoMerge(method) => (
            "enablePullRequestAutoMerge",
            "mutation($id: ID!, $method: PullRequestMergeMethod!) { enablePullRequestAutoMerge(input: { pullRequestId: $id, mergeMethod: $method }) { pullRequest { id } } }".to_string(),
            serde_json::json!({ "id": ids.id, "method": merge_method(*method) }),
        ),
        PullRequestAction::DisableAutoMerge => by_id("disablePullRequestAutoMerge"),
        PullRequestAction::Ready => by_id("markPullRequestReadyForReview"),
        PullRequestAction::Draft => by_id("convertPullRequestToDraft"),
        PullRequestAction::Close => by_id("closePullRequest"),
        PullRequestAction::Reopen => by_id("reopenPullRequest"),
        PullRequestAction::UpdateBranch { rebase } => (
            "updatePullRequestBranch",
            "mutation($id: ID!, $method: PullRequestBranchUpdateMethod!) { updatePullRequestBranch(input: { pullRequestId: $id, updateMethod: $method }) { pullRequest { id } } }".to_string(),
            serde_json::json!({ "id": ids.id, "method": if *rebase { "REBASE" } else { "MERGE" } }),
        ),
        PullRequestAction::DeleteBranch => {
            let Some(head_ref) = ids.head_ref else {
                return Err(refused("The pull request's branch is already gone."));
            };
            (
                "deleteRef",
                "mutation($id: ID!) { deleteRef(input: { refId: $id }) { clientMutationId } }"
                    .to_string(),
                serde_json::json!({ "id": head_ref.id }),
            )
        }
    };
    mutate(api, field, &query, variables).await
}

/// The repository's labels, each marked with whether `owner/repo#number`
/// carries it.
pub async fn label_candidates(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<Candidates<PullRequestLabelCandidate>, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    let data: LabelCandidatesData = decode(
        api.graphql(
            LABEL_CANDIDATES_QUERY,
            serde_json::json!({ "owner": owner, "name": repo, "number": number, "first": CANDIDATE_PAGE }),
        )
        .await?,
    )?;
    let repository = data
        .repository
        .ok_or_else(|| refused("GitHub has no such repository"))?;
    let applied: Vec<String> = repository.pull_request.map_or_default(|pr| {
        pr.labels
            .nodes
            .into_iter()
            .flatten()
            .map(|l| l.name)
            .collect()
    });
    let candidates: Vec<PullRequestLabelCandidate> = repository
        .labels
        .nodes
        .into_iter()
        .flatten()
        .map(|label| PullRequestLabelCandidate {
            is_applied: applied.contains(&label.name),
            name: label.name,
            color: label.color,
            description: label.description.filter(|text| !text.trim().is_empty()),
        })
        .collect();
    Ok(Candidates {
        truncated: repository.labels.total_count > candidates.len() as u64,
        candidates,
    })
}

/// The people `owner/repo#number` could be asked to review, matching `query`.
///
/// Its assignable users, less its author, whom GitHub refuses. Teams are not
/// listed: reading them needs organization access a
/// personal credential often lacks.
pub async fn reviewer_candidates(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    number: u64,
    query: Option<&str>,
) -> Result<Candidates<PullRequestReviewerCandidate>, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    let data: ReviewerCandidatesData = decode(
        api.graphql(
            REVIEWER_CANDIDATES_QUERY,
            serde_json::json!({
                "owner": owner, "name": repo, "number": number,
                "first": CANDIDATE_PAGE, "query": query,
            }),
        )
        .await?,
    )?;
    let repository = data
        .repository
        .ok_or_else(|| refused("GitHub has no such repository"))?;
    let (author, requested) = match repository.pull_request {
        Some(pr) => (
            pr.author.map(|author| author.login),
            pr.review_requests
                .nodes
                .into_iter()
                .flatten()
                .filter_map(|request| request.requested_reviewer?.login)
                .collect::<Vec<_>>(),
        ),
        None => (None, Vec::new()),
    };
    let listed = repository.assignable_users.nodes.len() as u64;
    let candidates = repository
        .assignable_users
        .nodes
        .into_iter()
        .flatten()
        .filter(|user| author.as_deref() != Some(user.login.as_str()))
        .map(|user| PullRequestReviewerCandidate {
            is_requested: requested.contains(&user.login),
            actor: PullRequestActor {
                login: user.login,
                name: user.name.filter(|name| !name.trim().is_empty()),
                avatar_url: user.avatar_url,
                is_bot: false,
            },
        })
        .collect();
    Ok(Candidates {
        truncated: repository.assignable_users.total_count > listed,
        candidates,
    })
}

/// The body of `POST /pulls/{number}/reviews`.
fn review_submission(
    verdict: ReviewVerdict,
    body: &str,
    comments: &[ReviewCommentDraft],
) -> serde_json::Value {
    let event = match verdict {
        ReviewVerdict::Comment => "COMMENT",
        ReviewVerdict::Approve => "APPROVE",
        ReviewVerdict::RequestChanges => "REQUEST_CHANGES",
    };
    let side = |position: &ReviewPosition| match position.side {
        DiffSide::Left => "LEFT",
        DiffSide::Right => "RIGHT",
    };
    let comments: Vec<serde_json::Value> = comments
        .iter()
        .map(|comment| {
            let mut value = serde_json::json!({
                "path": comment.path,
                "line": comment.position.line,
                "side": side(&comment.position),
                "body": comment.body,
            });
            // GitHub refuses a start that is the end itself.
            if let Some(start) = comment
                .start
                .as_ref()
                .filter(|start| **start != comment.position)
            {
                value["start_line"] = serde_json::json!(start.line);
                value["start_side"] = serde_json::json!(side(start));
            }
            value
        })
        .collect();
    serde_json::json!({ "event": event, "body": body, "comments": comments })
}

fn merge_method(method: MergeMethod) -> &'static str {
    match method {
        MergeMethod::Merge => "MERGE",
        MergeMethod::Squash => "SQUASH",
        MergeMethod::Rebase => "REBASE",
    }
}

/// GitHub's `ReactionContent` for T3's name of it.
fn reaction_content(content: &str) -> Result<String, AdapterError> {
    if !REACTIONS.contains(&content) {
        return Err(refused(&format!("Unknown reaction {content}")));
    }
    Ok(content.replace('-', "_").to_ascii_uppercase())
}

/// Run a one-field mutation. GitHub answers a refused mutation with `errors`
/// beside a `null` field rather than without `data`, which `graphql` passes
/// as usable, so the field is checked here.
async fn mutate(
    api: &GithubPrApi,
    field: &str,
    query: &str,
    variables: serde_json::Value,
) -> Result<(), AdapterError> {
    let data = api.graphql(query, variables).await?;
    if data.get(field).is_none_or(serde_json::Value::is_null) {
        return Err(refused(&format!("GitHub refused {field}")));
    }
    Ok(())
}

async fn node_ids(
    api: &GithubPrApi,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<NodeIds, AdapterError> {
    pull_request_data(
        api,
        NODE_IDS_QUERY,
        serde_json::json!({ "owner": owner, "name": repo, "number": number }),
    )
    .await
}

/// `text` percent-encoded as one URL path segment.
fn path_segment(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn refused(message: &str) -> AdapterError {
    SharedAdapterError::AdapterApi(message.to_string()).into()
}

fn decode<T: serde::de::DeserializeOwned>(data: serde_json::Value) -> Result<T, AdapterError> {
    serde_json::from_value(data).map_err(|e| refused(&format!("GitHub answered unexpectedly: {e}")))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeIds {
    id: String,
    head_ref: Option<NodeId>,
}

#[derive(Deserialize)]
struct NodeId {
    id: String,
}

#[derive(Deserialize)]
struct LabelCandidatesData {
    repository: Option<LabelCandidatesRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LabelCandidatesRepository {
    labels: Counted<RawLabel>,
    pull_request: Option<AppliedLabels>,
}

#[derive(Deserialize)]
struct AppliedLabels {
    labels: Nodes<RawLabel>,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Counted<T> {
    #[serde(default)]
    total_count: u64,
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<T>>,
}

#[derive(Deserialize)]
struct ReviewerCandidatesData {
    repository: Option<ReviewerCandidatesRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewerCandidatesRepository {
    assignable_users: Counted<RawUser>,
    pull_request: Option<RequestedReviewers>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawUser {
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestedReviewers {
    author: Option<RawLogin>,
    review_requests: Nodes<RawRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRequest {
    requested_reviewer: Option<RawLoginMaybe>,
}

#[derive(Deserialize)]
struct RawLogin {
    login: String,
}

/// A team or mannequin carries no login.
#[derive(Deserialize)]
struct RawLoginMaybe {
    login: Option<String>,
}

#[cfg(test)]
#[path = "pull_request_writes_tests.rs"]
mod tests;
