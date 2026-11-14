//! The pull request page's writes — comments, reviews, reactions, labels,
//! reviewers and the merge buttons — and the reads that serve its menus and
//! panels: `checks`, `stack`, `changeBreakdown` and the candidate lists.
//!
//! A write is authorized as a read is, through [`reach`], and is then made
//! with the viewer's own git-host credential per
//! [`crate::ports::PullRequestReviewSource::write`].

use serde::Deserialize;
use serde_json::{Value, json};

use moatless_types::access_control::ScopeContext;
use moatless_types::pull_request_conversation::{
    DiffSide, MergeMethod, PullRequestAction, PullRequestWrite, ReviewCommentDraft, ReviewPosition,
    ReviewVerdict,
};

use crate::changed_file_kind::{ChangedFileKind, classify};
use crate::pull_request_conversation::{actor_json, check_json};
use crate::pull_request_review::{
    PullRequestRef, PullRequestReviewError, PullRequestTarget, ReviewPorts, reach,
};

/// What a viewer with no GitHub credential of their own is told.
const NO_CREDENTIAL: &str = "Connect your GitHub account in Settings → Version control to act on pull requests as yourself.";

/// `pullRequests.checks`: the head commit's checks and the pull request's
/// state.
pub(crate) async fn checks(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
) -> Result<Value, PullRequestReviewError> {
    let workspace_id = reach(ports, viewer, target).await?;
    let workspace_id = workspace_id.as_deref();
    let (overview, status) = futures::join!(
        ports
            .host
            .overview(viewer, workspace_id, target.repository, target.number),
        ports
            .host
            .status(viewer, workspace_id, target.repository, target.number),
    );
    Ok(json!({
        "observedAt": chrono::Utc::now().timestamp_millis(),
        "state": overview?.state,
        "checks": status?.checks.iter().map(check_json).collect::<Vec<_>>(),
    }))
}

/// `pullRequests.changeBreakdown`: the change's lines by kind of file, or
/// `null` where the host lists too many files to count.
pub(crate) async fn change_breakdown(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
) -> Result<Value, PullRequestReviewError> {
    let workspace_id = reach(ports, viewer, target).await?;
    let Some(files) = ports
        .host
        .file_stats(
            viewer,
            workspace_id.as_deref(),
            target.repository,
            target.number,
        )
        .await?
    else {
        return Ok(Value::Null);
    };
    let mut breakdown = json!({});
    for kind in ["code", "tests", "mocks", "generated", "docs"] {
        breakdown[kind] = json!({ "additions": 0, "deletions": 0 });
    }
    for file in files {
        let bucket = &mut breakdown[match classify(&file.path) {
            ChangedFileKind::Code => "code",
            ChangedFileKind::Tests => "tests",
            ChangedFileKind::Mocks => "mocks",
            ChangedFileKind::Generated => "generated",
            ChangedFileKind::Docs => "docs",
        }];
        for (field, lines) in [("additions", file.additions), ("deletions", file.deletions)] {
            bucket[field] = json!(bucket[field].as_u64().unwrap_or(0) + lines);
        }
    }
    Ok(breakdown)
}

/// `pullRequests.labelCandidates`.
pub(crate) async fn label_candidates(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
) -> Result<Value, PullRequestReviewError> {
    authorize(ports, viewer, target).await?;
    let found = ports
        .host
        .label_candidates(viewer, target.repository, target.number)
        .await?;
    Ok(json!({
        "candidates": found.candidates.iter().map(|label| json!({
            "name": label.name,
            "color": label.color,
            "description": label.description,
            "isApplied": label.is_applied,
        })).collect::<Vec<_>>(),
        "truncated": found.truncated,
    }))
}

/// `pullRequests.reviewerCandidates`.
pub(crate) async fn reviewer_candidates(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
    query: Option<&str>,
) -> Result<Value, PullRequestReviewError> {
    authorize(ports, viewer, target).await?;
    let found = ports
        .host
        .reviewer_candidates(viewer, target.repository, target.number, query)
        .await?;
    Ok(json!({
        "candidates": found.candidates.iter().map(|candidate| {
            let mut value = actor_json(&candidate.actor);
            value["id"] = json!(candidate.actor.login);
            value["kind"] = json!("user");
            value["isRequested"] = json!(candidate.is_requested);
            value
        }).collect::<Vec<_>>(),
        "truncated": found.truncated,
    }))
}

/// Make `write` as the viewer.
pub(crate) async fn write(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
    write: &PullRequestWrite,
) -> Result<(), PullRequestReviewError> {
    authorize(ports, viewer, target).await?;
    ports
        .host
        .write(viewer, target.repository, target.number, write)
        .await?;
    Ok(())
}

/// `pullRequests.runAction`: make `action`, then answer the pull request as
/// it now stands, which the client shows without another read.
pub(crate) async fn run_action(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
    action: PullRequestAction,
) -> Result<Value, PullRequestReviewError> {
    write(ports, viewer, target, &PullRequestWrite::Action(action)).await?;
    let pr = ports
        .host
        .overview(viewer, None, target.repository, target.number)
        .await?;
    let mut summary = json!({
        "provider": "github",
        "projectId": target.project_id,
        "repository": target.repository,
        "number": pr.number,
        "title": if pr.title.trim().is_empty() { format!("#{}", pr.number) } else { pr.title.clone() },
        "url": pr.url,
        "state": pr.state,
        "isDraft": pr.is_draft,
        "headBranch": non_empty_or(&pr.head_ref, &pr.head_sha),
        "baseBranch": non_empty_or(&pr.base_ref, &pr.base_sha),
        "closedAt": pr.closed_at,
        "mergedAt": pr.merged_at,
        "additions": pr.additions,
        "deletions": pr.deletions,
        "changedFiles": pr.changed_files,
        "observedAt": chrono::Utc::now().timestamp_millis(),
    });
    if let Some(updated_at) = pr.updated_at.as_ref().or(pr.created_at.as_ref()) {
        summary["updatedAt"] = json!(updated_at);
    }
    if !pr.head_sha.trim().is_empty() {
        summary["headSha"] = json!(pr.head_sha);
    }
    Ok(summary)
}

fn non_empty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

/// The viewer may reach the pull request and has a credential of their own
/// to act with.
async fn authorize(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
) -> Result<(), PullRequestReviewError> {
    reach(ports, viewer, target).await?;
    if ports.auth.github_identity(viewer).await.is_none() {
        return Err(PullRequestReviewError::Refused(NO_CREDENTIAL.to_string()));
    }
    Ok(())
}

/// The body of every write method: the pull request it names and what is
/// written. Every field is optional so a missing one is answered with a typed
/// refusal, as [`PullRequestRef`]'s are.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WriteInput {
    #[serde(flatten)]
    pub(crate) reference: PullRequestRef,
    body: Option<String>,
    title: Option<String>,
    maintainer_can_modify: Option<bool>,
    thread_id: Option<String>,
    resolved: Option<bool>,
    comment_id: Option<String>,
    kind: Option<String>,
    subject_id: Option<String>,
    content: Option<String>,
    reacted: Option<bool>,
    verdict: Option<String>,
    #[serde(default)]
    comments: Vec<ReviewCommentInput>,
    #[serde(default)]
    labels: Vec<String>,
    applied: Option<bool>,
    #[serde(default)]
    reviewers: Vec<ReviewerInput>,
    requested: Option<bool>,
    action: Option<String>,
    merge_method: Option<String>,
    update_method: Option<String>,
    query: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewCommentInput {
    path: String,
    position: PositionInput,
    start_position: Option<PositionInput>,
    body: String,
}

/// The contract's `PullRequestReviewPosition`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PositionInput {
    kind: String,
    old_line: Option<u64>,
    new_line: Option<u64>,
    side: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReviewerInput {
    id: String,
    kind: String,
}

/// Which write method a request is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteMethod {
    Comment,
    ReplyToThread,
    SetThreadResolution,
    SubmitReview,
    SetReaction,
    Update,
    UpdateComment,
    SetLabels,
    RequestReviewers,
    RunAction,
}

impl WriteInput {
    /// The candidate search text, trimmed; `None` when blank.
    pub(crate) fn query(&self) -> Option<&str> {
        self.query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
    }

    /// What `method` writes, or why the input names nothing to write.
    pub(crate) fn into_write(self, method: WriteMethod) -> Result<PullRequestWrite, &'static str> {
        let body = |body: Option<String>| body.filter(|body| !body.is_empty());
        let id = |id: Option<String>| id.filter(|id| !id.trim().is_empty());
        Ok(match method {
            WriteMethod::Comment => PullRequestWrite::Comment {
                body: body(self.body).ok_or("A comment needs a body.")?,
            },
            WriteMethod::ReplyToThread => PullRequestWrite::ReplyToThread {
                thread_id: id(self.thread_id).ok_or("A reply needs a threadId.")?,
                body: body(self.body).ok_or("A reply needs a body.")?,
            },
            WriteMethod::SetThreadResolution => PullRequestWrite::SetThreadResolution {
                thread_id: id(self.thread_id).ok_or("A resolution needs a threadId.")?,
                resolved: self.resolved.ok_or("A resolution needs resolved.")?,
            },
            WriteMethod::SubmitReview => PullRequestWrite::SubmitReview {
                verdict: match self.verdict.as_deref() {
                    Some("comment") => ReviewVerdict::Comment,
                    Some("approve") => ReviewVerdict::Approve,
                    Some("request-changes") => ReviewVerdict::RequestChanges,
                    _ => return Err("A review needs a verdict."),
                },
                body: self.body.unwrap_or_default(),
                comments: self
                    .comments
                    .into_iter()
                    .map(ReviewCommentInput::into_draft)
                    .collect::<Option<_>>()
                    .ok_or("A review comment names no line.")?,
            },
            WriteMethod::SetReaction => PullRequestWrite::SetReaction {
                subject_id: id(self.subject_id),
                content: self.content.ok_or("A reaction needs a content.")?,
                reacted: self.reacted.ok_or("A reaction needs reacted.")?,
            },
            WriteMethod::Update => {
                if self.title.is_none()
                    && self.body.is_none()
                    && self.maintainer_can_modify.is_none()
                {
                    return Err("An update needs a title, body or maintainerCanModify.");
                }
                PullRequestWrite::Update {
                    title: self.title,
                    body: self.body,
                    maintainer_can_modify: self.maintainer_can_modify,
                }
            }
            WriteMethod::UpdateComment => PullRequestWrite::UpdateComment {
                comment_id: id(self.comment_id).ok_or("An edit needs a commentId.")?,
                review_comment: match self.kind.as_deref() {
                    Some("review-comment") => true,
                    Some("issue-comment") => false,
                    _ => return Err("An edit needs a kind."),
                },
                body: body(self.body).ok_or("An edit needs a body.")?,
            },
            WriteMethod::SetLabels => {
                if self.labels.is_empty() {
                    return Err("A label change needs labels.");
                }
                PullRequestWrite::SetLabels {
                    labels: self.labels,
                    applied: self.applied.ok_or("A label change needs applied.")?,
                }
            }
            WriteMethod::RequestReviewers => {
                if self.reviewers.is_empty() {
                    return Err("A review request needs reviewers.");
                }
                let (teams, users): (Vec<_>, Vec<_>) = self
                    .reviewers
                    .into_iter()
                    .partition(|reviewer| reviewer.kind == "team");
                PullRequestWrite::RequestReviewers {
                    users: users.into_iter().map(|reviewer| reviewer.id).collect(),
                    // A team is shown as `org/slug`; GitHub takes the slug.
                    teams: teams
                        .into_iter()
                        .map(|team| match team.id.rsplit_once('/') {
                            Some((_, slug)) => slug.to_string(),
                            None => team.id,
                        })
                        .collect(),
                    requested: self.requested.ok_or("A review request needs requested.")?,
                }
            }
            WriteMethod::RunAction => PullRequestWrite::Action(self.action()?),
        })
    }

    fn action(&self) -> Result<PullRequestAction, &'static str> {
        let merge_method = || match self.merge_method.as_deref() {
            None | Some("merge") => Ok(MergeMethod::Merge),
            Some("squash") => Ok(MergeMethod::Squash),
            Some("rebase") => Ok(MergeMethod::Rebase),
            Some(_) => Err("GitHub cannot merge that way here."),
        };
        Ok(match self.action.as_deref() {
            Some("merge") => PullRequestAction::Merge(merge_method()?),
            Some("enable-auto-merge") => PullRequestAction::EnableAutoMerge(merge_method()?),
            Some("disable-auto-merge") => PullRequestAction::DisableAutoMerge,
            Some("ready") => PullRequestAction::Ready,
            Some("draft") => PullRequestAction::Draft,
            Some("close") => PullRequestAction::Close,
            Some("reopen") => PullRequestAction::Reopen,
            Some("delete-branch") => PullRequestAction::DeleteBranch,
            Some("update-branch") => PullRequestAction::UpdateBranch {
                rebase: self.update_method.as_deref() == Some("rebase"),
            },
            _ => return Err("This action is not offered here."),
        })
    }
}

impl ReviewCommentInput {
    fn into_draft(self) -> Option<ReviewCommentDraft> {
        Some(ReviewCommentDraft {
            path: self.path,
            position: self.position.resolve()?,
            start: match self.start_position {
                Some(start) => Some(start.resolve()?),
                None => None,
            },
            body: self.body,
        })
    }
}

impl PositionInput {
    /// The line and side GitHub anchors a comment on: an added line on the
    /// new side, a deleted one on the old, context on the side it was made.
    fn resolve(&self) -> Option<ReviewPosition> {
        let (line, side) = match (self.kind.as_str(), self.side.as_deref()) {
            ("added", _) => (self.new_line?, DiffSide::Right),
            ("deleted", _) => (self.old_line?, DiffSide::Left),
            ("context", Some("left")) => (self.old_line?, DiffSide::Left),
            ("context", _) => (self.new_line?, DiffSide::Right),
            _ => return None,
        };
        Some(ReviewPosition { line, side })
    }
}

#[cfg(test)]
#[path = "pull_request_writes_tests.rs"]
mod tests;
