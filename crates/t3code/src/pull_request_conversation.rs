//! The reads of the T3 pull request page beyond its detail and diff —
//! `pullRequests.activity`, `threadComments`, `diffFileContents`, `preview`
//! and `linkedThreads` — authorized as [`crate::pull_request_review`] does.

use moatless_types::access_control::ScopeContext;
use moatless_types::pull_request_conversation::{
    DiffSide, FileChangeType, PullRequestActivity, PullRequestActor, PullRequestCheck,
    PullRequestCheckStatus, PullRequestCommitEntry, PullRequestLabel, PullRequestReaction,
    PullRequestRemark, PullRequestRemarkKind, ReviewThread, ReviewThreadComment,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ports::ChangedFile;
use crate::pull_request_review::{
    PullRequestRef, PullRequestReviewError, PullRequestTarget, ReviewPorts, non_empty_or,
    now_rfc3339, reach, rfc3339,
};

/// The contract's `PullRequestThreadCommentsInput`. Its fields are optional, as
/// [`PullRequestRef`]'s are, so a missing one gets the typed refusal.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PullRequestThreadCommentsInput {
    #[serde(flatten)]
    pub(crate) reference: PullRequestRef,
    pub(crate) thread_id: Option<String>,
    pub(crate) cursor: Option<String>,
}

/// The contract's `PullRequestDiffFileContentsInput`, optional as
/// [`PullRequestThreadCommentsInput`]'s fields are.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PullRequestDiffFileContentsInput {
    #[serde(flatten)]
    pub(crate) reference: PullRequestRef,
    pub(crate) commit: Option<String>,
    pub(crate) change_type: Option<String>,
    pub(crate) old_path: Option<String>,
    pub(crate) new_path: Option<String>,
}

/// `pullRequests.activity`: the conversation, review threads and commits.
pub(crate) async fn activity(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
) -> Result<Value, PullRequestReviewError> {
    let workspace_id = reach(ports, viewer, target).await?;
    let activity = ports
        .host
        .activity(
            viewer,
            workspace_id.as_deref(),
            target.repository,
            target.number,
        )
        .await?;
    Ok(activity_json(&activity))
}

/// `pullRequests.threadComments`: the page after `cursor` of one thread.
pub(crate) async fn thread_comments(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
    thread_id: &str,
    cursor: &str,
) -> Result<Value, PullRequestReviewError> {
    let workspace_id = reach(ports, viewer, target).await?;
    let page = ports
        .host
        .thread_comments(
            viewer,
            workspace_id.as_deref(),
            target.repository,
            target.number,
            thread_id,
            Some(cursor),
        )
        .await?;
    Ok(json!({
        "comments": page.comments.iter().map(thread_comment_json).collect::<Vec<_>>(),
        "nextCursor": page.next_cursor,
    }))
}

/// `pullRequests.diffFileContents`: one changed file before and after.
pub(crate) async fn file_contents(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
    file: &ChangedFile<'_>,
) -> Result<Value, PullRequestReviewError> {
    let workspace_id = reach(ports, viewer, target).await?;
    let contents = ports
        .host
        .file_contents(
            viewer,
            workspace_id.as_deref(),
            target.repository,
            target.number,
            file,
        )
        .await?;
    Ok(json!({
        "oldContents": contents.old_contents,
        "newContents": contents.new_contents,
    }))
}

/// `pullRequests.preview`: the hover card.
pub(crate) async fn preview(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
) -> Result<Value, PullRequestReviewError> {
    let workspace_id = reach(ports, viewer, target).await?;
    let pr = ports
        .host
        .overview(
            viewer,
            workspace_id.as_deref(),
            target.repository,
            target.number,
        )
        .await?;
    let created_at = pr
        .created_at
        .clone()
        .or_else(|| pr.updated_at.clone())
        .unwrap_or_else(now_rfc3339);
    Ok(json!({
        "projectId": target.project_id,
        "repository": target.repository,
        "number": pr.number,
        "title": non_empty_or(&pr.title, &format!("#{}", pr.number)),
        "url": pr.url,
        "author": pr.author_login.as_ref().map(|login| json!({
            "login": login,
            "name": null,
            "avatarUrl": pr.author_avatar_url,
        })),
        "state": pr.state,
        "isDraft": pr.is_draft,
        "createdAt": created_at,
    }))
}

/// `pullRequests.linkedThreads`: the readable Tasks bound to the pull request.
/// A Task in no Workspace has no project to name and is left out.
pub(crate) async fn linked_threads(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
) -> Result<Value, PullRequestReviewError> {
    let task_ids = ports
        .bound
        .tasks_bound_to(target.repository, target.number)
        .await;
    let readable = if task_ids.is_empty() {
        Vec::new()
    } else {
        ports
            .conversations
            .readable_by_ids(viewer, &task_ids)
            .await?
    };
    let threads: Vec<Value> = readable
        .iter()
        .filter_map(|conversation| {
            Some(json!({
                "id": conversation.task_id,
                "projectId": conversation.workspace_id.as_deref()?,
                "title": conversation
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .unwrap_or("New thread"),
                "archivedAt": conversation.closed_at.map(rfc3339),
            }))
        })
        .collect();
    Ok(json!({ "threads": threads }))
}

/// The `ChangedFile` a `pullRequests.diffFileContents` request names.
pub(crate) fn changed_file(input: &PullRequestDiffFileContentsInput) -> Option<ChangedFile<'_>> {
    fn text(value: &Option<String>) -> Option<&str> {
        value.as_deref().filter(|value| !value.trim().is_empty())
    }
    Some(ChangedFile {
        commit: text(&input.commit),
        change: match text(&input.change_type)? {
            "change" => FileChangeType::Change,
            "rename-pure" => FileChangeType::RenamePure,
            "rename-changed" => FileChangeType::RenameChanged,
            "new" => FileChangeType::New,
            "deleted" => FileChangeType::Deleted,
            _ => return None,
        },
        old_path: text(&input.old_path)?,
        new_path: text(&input.new_path)?,
    })
}

/// Whether `commit` is a commit id rather than a ref a reader could aim
/// anywhere in the repository.
pub(crate) fn is_commit_sha(commit: &str) -> bool {
    (7..=64).contains(&commit.len()) && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn activity_json(activity: &PullRequestActivity) -> Value {
    json!({
        "author": activity.author.as_ref().map(actor_json),
        "reviewers": activity.reviewers.iter().map(actor_json).collect::<Vec<_>>(),
        "comments": activity.remarks.iter().map(remark_json).collect::<Vec<_>>(),
        "commentCount": activity.remark_count,
        "commentsTruncated": activity.remarks_truncated,
        "reviewThreadsTruncated": activity.review_threads_truncated,
        "reviewThreads": activity.review_threads.iter().map(thread_json).collect::<Vec<_>>(),
        "commits": activity.commits.iter().map(commit_json).collect::<Vec<_>>(),
        "reactions": reactions_json(&activity.reactions),
    })
}

fn reactions_json(reactions: &[PullRequestReaction]) -> Vec<Value> {
    reactions
        .iter()
        .map(|reaction| {
            json!({
                "content": reaction.content,
                "count": reaction.count,
                "actors": reaction.actors,
                "viewerHasReacted": reaction.viewer_has_reacted,
            })
        })
        .collect()
}

/// Sets `canEdit` on `value` where the host said.
fn with_can_edit(mut value: Value, can_edit: Option<bool>) -> Value {
    if let Some(can_edit) = can_edit {
        value["canEdit"] = json!(can_edit);
    }
    value
}

pub(crate) fn actor_json(actor: &PullRequestActor) -> Value {
    let mut value = json!({
        "login": actor.login,
        "name": actor.name,
        "avatarUrl": actor.avatar_url,
    });
    if actor.is_bot {
        value["isBot"] = json!(true);
    }
    value
}

pub(crate) fn label_json(label: &PullRequestLabel) -> Value {
    json!({ "name": label.name, "color": label.color })
}

pub(crate) fn check_json(check: &PullRequestCheck) -> Value {
    let mut value = json!({
        "name": check.name,
        "status": match check.status {
            PullRequestCheckStatus::Pending => "pending",
            PullRequestCheckStatus::ActionRequired => "action-required",
            PullRequestCheckStatus::Success => "success",
            PullRequestCheckStatus::Failure => "failure",
            PullRequestCheckStatus::Skipped => "skipped",
            PullRequestCheckStatus::Neutral => "neutral",
            PullRequestCheckStatus::Cancelled => "cancelled",
        },
        "description": check.description,
        "url": check.url,
    });
    if let Some(required) = check.required {
        value["required"] = json!(required);
    }
    value
}

fn remark_json(remark: &PullRequestRemark) -> Value {
    let value = json!({
        "id": remark.id,
        "kind": match remark.kind {
            PullRequestRemarkKind::IssueComment => "issue-comment",
            PullRequestRemarkKind::ReviewComment => "review-comment",
            PullRequestRemarkKind::Review => "review",
        },
        "author": remark.author.as_ref().map(actor_json),
        "body": remark.body,
        "createdAt": remark.created_at,
        "editedAt": remark.edited_at,
        "url": remark.url,
        "path": remark.path,
        "reviewState": remark.review_state,
        "reactions": reactions_json(&remark.reactions),
    });
    with_can_edit(value, remark.can_edit)
}

fn thread_json(thread: &ReviewThread) -> Value {
    let mut value = json!({
        "id": thread.id,
        "path": thread.path,
        "line": thread.line,
        "side": match thread.side {
            DiffSide::Left => "left",
            DiffSide::Right => "right",
        },
        "isResolved": thread.is_resolved,
        "isOutdated": thread.is_outdated,
        "comments": thread.comments.iter().map(thread_comment_json).collect::<Vec<_>>(),
        "commentCount": thread.comment_count,
    });
    if let Some(cursor) = &thread.next_comments_cursor {
        value["nextCommentsCursor"] = json!(cursor);
    }
    if let Some(can_resolve) = thread.can_resolve {
        value["canResolve"] = json!(can_resolve);
    }
    value
}

fn thread_comment_json(comment: &ReviewThreadComment) -> Value {
    let value = json!({
        "id": comment.id,
        "author": comment.author.as_ref().map(actor_json),
        "body": comment.body,
        "createdAt": comment.created_at,
        "editedAt": comment.edited_at,
        "url": comment.url,
        "reactions": reactions_json(&comment.reactions),
    });
    with_can_edit(value, comment.can_edit)
}

fn commit_json(commit: &PullRequestCommitEntry) -> Value {
    let mut value = json!({
        "oid": commit.oid,
        "messageHeadline": commit.message_headline,
        "committedDate": commit.committed_date,
        "authors": commit.authors.iter().map(actor_json).collect::<Vec<_>>(),
    });
    if let Some(additions) = commit.additions {
        value["additions"] = json!(additions);
    }
    if let Some(deletions) = commit.deletions {
        value["deletions"] = json!(deletions);
    }
    value
}

#[cfg(test)]
#[path = "pull_request_conversation_tests.rs"]
mod tests;
