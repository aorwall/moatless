//! A pull request read from its git host: `pullRequests.detail` and its diff.
//!
//! Every read is authorized through the project it names, which must place
//! the pull request's repository, or else through a Task the viewer
//! may read that is bound to it: an arbitrary reference answers not found.

use chrono::{DateTime, Utc};
use moatless_types::access_control::ScopeContext;
use moatless_types::pull_request_conversation::{PullRequestStatus, PullRequestViewerRights};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::dao::{Conversation, ConversationDao};
use crate::nominal_path;
use crate::ports::{
    BoundPullRequestSource, PullRequestError, PullRequestOverview, PullRequestReviewSource,
    SourceControlAuthSource,
};
use crate::pull_request_conversation::{actor_json, check_json, label_json};
use crate::review::{PullRequestSummaryError, readable_binding};
use crate::workspace_listing::WorkspaceListing;

/// Why a pull request could not be read or written.
#[derive(Debug)]
pub(crate) enum PullRequestReviewError {
    /// No readable Task is bound to the reference. One answer for every cause,
    /// as [`PullRequestSummaryError::Unknown`] is.
    Unknown,
    Store(sqlx::Error),
    Host(PullRequestError),
    /// The write cannot be made; the message says why.
    Refused(String),
}

impl From<PullRequestSummaryError> for PullRequestReviewError {
    fn from(error: PullRequestSummaryError) -> Self {
        match error {
            PullRequestSummaryError::Unknown => Self::Unknown,
            PullRequestSummaryError::Store(error) => Self::Store(error),
        }
    }
}

impl From<sqlx::Error> for PullRequestReviewError {
    fn from(error: sqlx::Error) -> Self {
        Self::Store(error)
    }
}

impl From<PullRequestError> for PullRequestReviewError {
    fn from(error: PullRequestError) -> Self {
        match error {
            PullRequestError::NotFound => Self::Unknown,
            other => Self::Host(other),
        }
    }
}

/// What a request names: the project it was opened in and the pull request.
pub(crate) struct PullRequestTarget<'a> {
    pub(crate) project_id: &'a str,
    pub(crate) repository: &'a str,
    pub(crate) number: u64,
}

/// The ports and reads every function here takes.
pub(crate) struct ReviewPorts<'a> {
    pub(crate) conversations: &'a dyn ConversationDao,
    pub(crate) workspaces: &'a dyn WorkspaceListing,
    pub(crate) bound: &'a dyn BoundPullRequestSource,
    pub(crate) host: &'a dyn PullRequestReviewSource,
    pub(crate) auth: &'a dyn SourceControlAuthSource,
}

/// The contract's `PullRequestRef`, as far as it names a pull request.
///
/// Every field is optional here so that a missing one is answered with the
/// caller's typed refusal rather than refused by the decode as a defect.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PullRequestRef {
    project_id: Option<String>,
    repository: Option<String>,
    number: Option<u64>,
}

impl PullRequestRef {
    /// The project, repository and number, or `None` when one is missing,
    /// blank or zero.
    pub(crate) fn target(&self) -> Option<(String, String, u64)> {
        let text = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let number = self.number.filter(|number| *number != 0)?;
        Some((text(&self.project_id)?, text(&self.repository)?, number))
    }
}

/// The pull request a `PullRequestRef`-shaped body names; see
/// [`PullRequestRef::target`].
pub(crate) fn pull_request_ref(payload: &Value) -> Option<(String, String, u64)> {
    PullRequestRef::deserialize(payload).ok()?.target()
}

/// The ports a request answers from.
pub(crate) fn review_ports(state: &crate::state::UiRpcState) -> ReviewPorts<'_> {
    ReviewPorts {
        conversations: state.reads.conversations(),
        workspaces: state.reads.workspaces(),
        bound: state.pull_requests.as_ref(),
        host: state.pull_request_reviews.as_ref(),
        auth: state.source_control_auth.as_ref(),
    }
}

/// The Workspace the viewer reaches `target` through: the named project when
/// it places the pull request's repository or a readable Task in it
/// works in, else that of a readable Task bound to it. `None` inside `Ok` is a
/// bound Task with no Workspace.
pub(crate) async fn reach(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
) -> Result<Option<String>, PullRequestReviewError> {
    reach_through(ports.host, ports.conversations, ports.bound, viewer, target).await
}

/// [`reach`], over only the ports it reads.
async fn reach_through(
    host: &dyn PullRequestReviewSource,
    conversations: &dyn ConversationDao,
    bound: &dyn BoundPullRequestSource,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
) -> Result<Option<String>, PullRequestReviewError> {
    if host
        .project_repositories(viewer, target.project_id)
        .await?
        .iter()
        .any(|repository| repository.eq_ignore_ascii_case(target.repository))
    {
        return Ok(Some(target.project_id.to_string()));
    }
    if host
        .task_repositories(viewer, target.project_id)
        .await?
        .iter()
        .any(|repository| repository.eq_ignore_ascii_case(target.repository))
    {
        return Ok(Some(target.project_id.to_string()));
    }
    readable_binding(
        conversations,
        bound,
        viewer,
        target.repository,
        target.number,
    )
    .await?
    .map(|(conversation, _): (Conversation, _)| conversation.workspace_id)
    .ok_or(PullRequestReviewError::Unknown)
}

/// `pullRequests.detail`: the pull request as its host reports it, and what
/// the viewer may do to it.
pub(crate) async fn detail(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
) -> Result<Value, PullRequestReviewError> {
    let workspace_id = reach(ports, viewer, target).await?;
    let (overview, status) = futures::join!(
        ports.host.overview(
            viewer,
            workspace_id.as_deref(),
            target.repository,
            target.number,
        ),
        ports.host.status(
            viewer,
            workspace_id.as_deref(),
            target.repository,
            target.number,
        ),
    );
    let overview = overview?;
    // The pull request reads without its labels, reviewers and checks rather
    // than not at all.
    let status = status.unwrap_or_else(|error| {
        tracing::warn!(?error, repository = %target.repository, number = target.number, "Pull request status read failed");
        PullRequestStatus::default()
    });
    let project_title = ports
        .workspaces
        .list_readable_including(viewer, &[target.project_id.to_string()])
        .await?
        .into_iter()
        .find(|listed| listed.workspace.id == target.project_id)
        .map(|listed| listed.workspace.name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| target.repository.to_string());
    Ok(detail_json(target, &project_title, &overview, &status))
}

fn detail_json(
    target: &PullRequestTarget<'_>,
    project_title: &str,
    pr: &PullRequestOverview,
    status: &PullRequestStatus,
) -> Value {
    let now = now_rfc3339();
    let created_at = pr.created_at.clone().unwrap_or_else(|| now.clone());
    let mut detail = json!({
        "provider": "github",
        "capabilities": capabilities_json(),
        "viewerPermissions": viewer_permissions_json(status.viewer.as_ref()),
        "projectId": target.project_id,
        "projectTitle": project_title,
        "workspaceRoot": nominal_path::nominal_root(target.project_id),
        // The reference the caller asked with, as `pullRequests.summary` does.
        "repository": target.repository,
        "number": pr.number,
        "title": non_empty_or(&pr.title, &format!("#{}", pr.number)),
        "body": pr.body,
        "url": pr.url,
        "author": pr.author_login.as_ref().map(|login| json!({
            "login": login,
            "name": null,
            "avatarUrl": pr.author_avatar_url,
        })),
        "state": pr.state,
        "isDraft": pr.is_draft,
        "mergeability": match pr.mergeable {
            Some(true) => "mergeable",
            Some(false) => "conflicting",
            None => "unknown",
        },
        "additions": pr.additions,
        "deletions": pr.deletions,
        "changedFiles": pr.changed_files,
        "headBranch": non_empty_or(&pr.head_ref, &pr.head_sha),
        "headRepositoryNameWithOwner": pr.head_repository,
        "baseBranch": non_empty_or(&pr.base_ref, &pr.base_sha),
        "updatedAt": pr.updated_at.clone().unwrap_or_else(|| created_at.clone()),
        "createdAt": created_at,
        "mergedAt": pr.merged_at,
        "closedAt": pr.closed_at,
        "reviewers": status.requested_reviewers.iter().map(actor_json).collect::<Vec<_>>(),
        "labels": status.labels.iter().map(label_json).collect::<Vec<_>>(),
        "checks": status.checks.iter().map(check_json).collect::<Vec<_>>(),
        "mergeCapabilities": {
            "merge": status.viewer.as_ref().is_some_and(|viewer| viewer.merge_commit_allowed),
            "squash": status.viewer.as_ref().is_some_and(|viewer| viewer.squash_merge_allowed),
            "rebase": status.viewer.as_ref().is_some_and(|viewer| viewer.rebase_merge_allowed),
        },
    });
    if !pr.head_sha.trim().is_empty() {
        detail["headSha"] = json!(pr.head_sha);
    }
    if let Some(viewer) = &status.viewer {
        detail["autoMergeEnabled"] = json!(viewer.auto_merge_method.is_some());
        if let Some(method) = &viewer.auto_merge_method {
            detail["autoMergeMethod"] = json!(method);
        }
    }
    detail
}

/// What this backend can do to a GitHub pull request at all, whoever asks:
/// `crate::pull_request_writes` serves each of these. Not `revert`,
/// `approve-workflows`, stacks or viewed files, which it does not serve.
fn capabilities_json() -> Value {
    json!({
        "diff": true,
        "comment": true,
        "actions": [
            "merge", "ready", "draft", "close", "reopen", "delete-branch",
            "update-branch", "enable-auto-merge", "disable-auto-merge",
        ],
        "mergeMethods": ["merge", "squash", "rebase"],
        "deleteBranch": true,
        "updateMethods": ["merge", "rebase"],
        "search": false,
        "reactions": true,
        "review": {
            "inlineComment": true,
            "rangeComment": true,
            "reply": true,
            "resolve": true,
            "verdicts": ["comment", "approve", "request-changes"],
        },
        "reviewers": { "request": true, "listCandidates": true, "searchCandidates": true },
        "edit": { "changeRequest": true, "comment": true },
        "stacks": false,
        "labels": true,
    })
}

/// What `viewer` may do, as upstream's `gitHubViewerPermissions` decides it;
/// nothing where the read did not say.
fn viewer_permissions_json(viewer: Option<&PullRequestViewerRights>) -> Value {
    let Some(viewer) = viewer else {
        return json!({
            "actions": [],
            "comment": false,
            "resolve": false,
            "verdicts": [],
            "requestReviewers": false,
        });
    };
    let mut actions = Vec::new();
    if viewer.can_delete_branch {
        actions.push("delete-branch");
    }
    if viewer.can_write {
        actions.push("merge");
        if viewer.auto_merge_allowed && !viewer.can_merge_now {
            actions.push("enable-auto-merge");
        }
        // Kept where the repository has since turned auto-merge off.
        actions.push("disable-auto-merge");
    }
    if viewer.can_update {
        actions.extend(["ready", "draft", "close", "reopen"]);
    }
    if viewer.can_update_branch {
        actions.push("update-branch");
    }
    let mut permissions = json!({
        "actions": actions,
        "comment": true,
        "resolve": viewer.can_write || viewer.did_author,
        // GitHub refuses an author's own approval or change request.
        "verdicts": if viewer.did_author {
            json!(["comment"])
        } else {
            json!(["comment", "approve", "request-changes"])
        },
        "requestReviewers": viewer.can_write,
        "labels": viewer.can_triage,
        "editChangeRequest": viewer.can_update,
    });
    if viewer.can_update_branch {
        permissions["updateMethods"] = json!(["merge", "rebase"]);
    }
    permissions
}

/// The HTTP `pull-requests/diff` route: one page of the change, or of its
/// `commit`. `cursor` is the page number a previous page handed out.
pub(crate) async fn diff(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    target: &PullRequestTarget<'_>,
    commit: Option<&str>,
    cursor: Option<&str>,
) -> Result<Value, PullRequestReviewError> {
    let workspace_id = reach(ports, viewer, target).await?;
    let page = cursor.and_then(|cursor| cursor.parse().ok()).unwrap_or(1);
    let page = match commit {
        Some(sha) => {
            ports
                .host
                .commit_patch_page(
                    viewer,
                    workspace_id.as_deref(),
                    target.repository,
                    sha,
                    page,
                )
                .await?
        }
        None => {
            ports
                .host
                .patch_page(
                    viewer,
                    workspace_id.as_deref(),
                    target.repository,
                    target.number,
                    page,
                )
                .await?
        }
    };
    Ok(json!({
        "patch": page.patch,
        "truncated": page.omitted_files > 0,
        "nextCursor": page.next_page.map(|page| page.to_string()),
    }))
}

pub(crate) fn non_empty_or(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

pub(crate) fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn now_rfc3339() -> String {
    rfc3339(Utc::now())
}

#[cfg(test)]
#[path = "pull_request_review_tests.rs"]
mod tests;
