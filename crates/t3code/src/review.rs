//! The diff-panel half of the T3 compatibility module.
//!
//! The panel reads a conversation's changes in two scopes — what is not
//! committed, and what the branch has committed since it left its base — picks
//! a base ref for the second, and shows which branch it is looking at. That is
//! three calls on the wire (`review.getDiffPreview`, `vcs.listRefs`,
//! `subscribeVcsStatus`), all addressed by [`super::workspace`]'s `cwd`, and
//! all answered from the conversation's Sandbox, which is the only place its
//! checkout exists.
//!
//! # What this surface does not carry
//!
//! The panel's other scope is a *turn checkpoint*: the worktree as it stood at
//! the end of one turn, diffed against the turn before. Moatless records no such
//! snapshot, so every checkpoint `projection::checkpoints` reports carries no
//! `files` and the panel offers the two git scopes alone. A checkpoint here
//! names a turn a person may rewind the *conversation* to, and nothing about the
//! working copy.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::dao::ConversationDao;
use super::nominal_path;
use super::ports::{
    BoundPullRequest, BoundPullRequestSource, BranchListError, RepositoryBranchSource, is_draft,
    primary_pull_request, t3_pr_state,
};
use super::workspace::{self, WorkspaceAccessError};
use crate::workspace_dao::T3WorkspaceDao;
use moatless_api_schemas::repository::{BranchEntry, BranchListResponse};
use moatless_ports::sandbox::SandboxError;
use moatless_types::access_control::ScopeContext;
use sandbox_proto::client::{SandboxConnector, classify_grpc_error};
use sandbox_proto::{GetGitDiffPreviewResponse, GetGitStatusResponse, ListGitRefsResponse};

/// How many refs one page carries when the client does not say, matching the
/// reference server's default.
const DEFAULT_REF_LIMIT: usize = 100;

/// Both diff scopes of one conversation, in the shape the contract's
/// `review.getDiffPreview` returns.
pub async fn diff_preview(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    cwd: &str,
    base_ref: Option<&str>,
    ignore_whitespace: bool,
) -> Result<Value, WorkspaceAccessError> {
    let task_id = workspace::addressed_conversation(conversations, viewer, cwd).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(workspace::sandbox_access)?;
    let preview = client
        .get_git_diff_preview_for_task(
            Some(&task_id),
            base_ref.map(str::to_string),
            ignore_whitespace,
        )
        .await
        .map_err(|status| {
            workspace::sandbox_access(classify_grpc_error(status, "get_git_diff_preview"))
        })?;

    Ok(json!({
        "cwd": cwd,
        "generatedAt": now_rfc3339(),
        "sources": sources(&preview),
    }))
}

/// The two sources the panel's scope selector switches between.
///
/// Both are always present, patch or no patch: the selector reads its options
/// off this list, and a scope that vanishes when it happens to be empty is a
/// scope a person cannot switch back to.
fn sources(preview: &GetGitDiffPreviewResponse) -> Value {
    let base_ref = single_value(preview.ranges.iter().map(|range| range.base_ref.as_str()));
    let head_ref = single_value(preview.ranges.iter().map(|range| range.head_ref.as_str()));
    json!([
        {
            "id": "working-tree",
            "kind": "working-tree",
            "title": "Dirty worktree",
            "baseRef": "HEAD",
            "headRef": Value::Null,
            "diff": preview.working_tree_diff,
            "diffHash": diff_hash(&preview.working_tree_diff),
            "truncated": preview.working_tree_truncated,
        },
        {
            "id": "branch-range",
            "kind": "branch-range",
            "title": match &base_ref {
                Some(base) => format!("Against {base}"),
                None => "Against base branch".to_string(),
            },
            "baseRef": nullable(base_ref),
            "headRef": nullable(head_ref),
            "diff": preview.branch_diff,
            "diffHash": diff_hash(&preview.branch_diff),
            "truncated": preview.branch_truncated,
        }
    ])
}

/// The one value every repository in the workspace agreed on, if they did.
///
/// The contract carries one ref name per source and a workspace can hold
/// several repositories. Naming one of their branches would put a ref in the
/// header that most of the patch was not taken against, so disagreement — and
/// a workspace with no repository at all — answers with nothing.
fn single_value<'a>(values: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut agreed: Option<&str> = None;
    for value in values {
        if value.is_empty() {
            return None;
        }
        match agreed {
            Some(existing) if existing != value => return None,
            _ => agreed = Some(value),
        }
    }
    agreed.map(str::to_string)
}

fn nullable(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::String)
}

/// A patch's identity, which the client uses to tell one render from the next.
fn diff_hash(diff: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(diff.as_bytes());
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// The primary repository's branches, in the shape the contract's
/// `vcs.listRefs` returns.
///
/// Filtering, ordering and paging happen here rather than in the Sandbox: the
/// Sandbox reads a repository's refs in one pass, and every knob the contract
/// offers is a view over that one list.
pub async fn list_refs(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    request: &RefRequest<'_>,
) -> Result<Value, WorkspaceAccessError> {
    let task_id = workspace::addressed_conversation(conversations, viewer, request.cwd).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(workspace::sandbox_access)?;
    let listing = client
        .list_git_refs_for_task(Some(&task_id))
        .await
        .map_err(|status| {
            workspace::sandbox_access(classify_grpc_error(status, "list_git_refs"))
        })?;

    Ok(ref_page(&listing, request))
}

/// What a conversation with no Sandbox to ask has to offer.
///
/// The client polls this in the background for every conversation it holds, so
/// "there is no repository reachable here" is answered rather than failed: a
/// failure is retried on a schedule and surfaces where nothing went wrong.
pub fn no_refs() -> Value {
    json!({
        "refs": [],
        "isRepo": false,
        "hasPrimaryRemote": false,
        "nextCursor": Value::Null,
        "totalCount": 0,
    })
}

/// Whether a failed ref listing should answer [`no_refs`] rather than fail.
///
/// The bar is whether the person could do anything with being told. A Sandbox
/// that is absent, stopped, or from an image older than the method is a state
/// of the conversation, and the client would surface it once per poll. A
/// Sandbox that is up and refusing, or a database that would not answer, is
/// this deployment failing, and hiding that behind an empty ref list would make
/// the branch picker look merely empty.
pub fn refs_are_quiet(error: &WorkspaceAccessError) -> bool {
    matches!(
        error,
        WorkspaceAccessError::NoWorktree
            | WorkspaceAccessError::Unaddressable
            | WorkspaceAccessError::NotRunning(_)
            | WorkspaceAccessError::Sandbox(SandboxError::Unsupported { .. })
    )
}

/// One `vcs.listRefs` request, as this surface reads it.
pub struct RefRequest<'a> {
    pub cwd: &'a str,
    /// Case-insensitive substring the ref's name must contain.
    pub query: Option<&'a str>,
    pub cursor: usize,
    pub limit: usize,
    /// Whether a remote ref is offered beside the local branch of the same
    /// name. The panel wants both, so it can show a branch and its upstream as
    /// one choice; a plain branch list wants one row per branch.
    pub include_matching_remote_refs: bool,
    pub ref_kind: RefKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefKind {
    All,
    Local,
    Remote,
}

impl RefKind {
    fn from_wire(value: Option<&str>) -> Self {
        match value {
            Some("local") => Self::Local,
            Some("remote") => Self::Remote,
            _ => Self::All,
        }
    }
}

impl<'a> RefRequest<'a> {
    /// Read one off the wire, with the contract's own defaults for what it
    /// leaves out.
    pub fn from_payload(cwd: &'a str, payload: &'a Value) -> Self {
        Self {
            cwd,
            query: payload
                .get("query")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|query| !query.is_empty()),
            cursor: payload
                .get("cursor")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .try_into()
                .unwrap_or(usize::MAX),
            limit: payload
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|limit| usize::try_from(limit).ok())
                .filter(|limit| *limit > 0)
                .unwrap_or(DEFAULT_REF_LIMIT),
            include_matching_remote_refs: payload
                .get("includeMatchingRemoteRefs")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            ref_kind: RefKind::from_wire(payload.get("refKind").and_then(Value::as_str)),
        }
    }
}

/// One page of refs, as the contract's result.
fn ref_page(listing: &ListGitRefsResponse, request: &RefRequest<'_>) -> Value {
    let local_names: std::collections::HashSet<&str> = listing
        .refs
        .iter()
        .filter(|reference| !reference.is_remote)
        .map(|reference| reference.name.as_str())
        .collect();

    let mut matching: Vec<&sandbox_proto::GitRef> = listing
        .refs
        .iter()
        .filter(|reference| match request.ref_kind {
            RefKind::All => true,
            RefKind::Local => !reference.is_remote,
            RefKind::Remote => reference.is_remote,
        })
        .filter(|reference| {
            request.include_matching_remote_refs || !shadows_a_local_branch(reference, &local_names)
        })
        .filter(|reference| match request.query {
            Some(query) => reference
                .name
                .to_lowercase()
                .contains(&query.to_lowercase()),
            None => true,
        })
        .collect();
    // The checked-out branch and the default one belong on the first page even
    // when the default exists only as `origin/<default>`, which would otherwise
    // sort after every local ref.
    matching.sort_by_key(
        |reference| match (reference.current, reference.is_default) {
            (true, _) => 0,
            (_, true) => 1,
            _ => 2,
        },
    );

    let total_count = matching.len();
    let page: Vec<Value> = matching
        .into_iter()
        .skip(request.cursor)
        .take(request.limit)
        .map(|reference| {
            json!({
                "name": reference.name,
                "isRemote": reference.is_remote,
                "remoteName": reference.remote_name,
                "current": reference.current,
                "isDefault": reference.is_default,
                // A Sandbox holds one checkout and no linked worktrees, so a
                // ref is never checked out anywhere but where it says.
                "worktreePath": Value::Null,
            })
        })
        .collect();
    let next_cursor = match request.cursor.saturating_add(page.len()) {
        reached if reached < total_count => json!(reached),
        _ => Value::Null,
    };

    json!({
        "refs": page,
        "isRepo": listing.is_repo,
        "hasPrimaryRemote": listing.has_primary_remote,
        "nextCursor": next_cursor,
        "totalCount": total_count,
    })
}

/// Whether this remote ref is the same branch as one already listed locally.
///
/// Only `origin` is folded away: a second remote's `main` is a different branch
/// from the local one, and hiding it would leave no way to diff against it.
fn shadows_a_local_branch(
    reference: &sandbox_proto::GitRef,
    local_names: &std::collections::HashSet<&str>,
) -> bool {
    if !reference.is_remote || reference.remote_name != "origin" {
        return false;
    }
    reference
        .name
        .strip_prefix("origin/")
        .is_some_and(|branch| local_names.contains(branch))
}

/// The branches of a draft thread's workspace, in the shape `vcs.listRefs`
/// returns.
///
/// A draft thread has no Sandbox, so it addresses its Workspace by the project
/// root rather than a worktree. Its branches are the primary repository's, read
/// from the git host — the same list Moatless's own branch picker shows before
/// a Task runs — rather than a checkout that does not exist yet. Access is
/// gated through the scoped workspace read first, so a project root the viewer
/// cannot be shown answers as an empty list.
pub async fn workspace_branches(
    workspaces: &dyn T3WorkspaceDao,
    branches: &dyn RepositoryBranchSource,
    viewer: &ScopeContext,
    cwd: &str,
    request: &RefRequest<'_>,
) -> Result<Value, BranchListError> {
    let workspace_id =
        nominal_path::workspace_id_from_project_root(cwd).ok_or(BranchListError::None)?;
    let Some(repository_id) = workspaces
        .primary_repository(viewer, workspace_id)
        .await
        .map_err(|e| {
            BranchListError::Failed(moatless_types::store::StoreError::with_source(
                format!("read the primary repository of workspace {workspace_id}"),
                e,
            ))
        })?
    else {
        return Err(BranchListError::None);
    };
    let response = branches
        .list_branches(viewer, &repository_id, Some(workspace_id))
        .await?;
    Ok(branch_ref_page(&response, request))
}

/// One page of a repository's branches, as the contract's `vcs.listRefs`
/// result.
///
/// The git host lists a repository's branches in one pass and every knob the
/// contract offers is a view over that one list — the same relationship
/// [`ref_page`] has with a Sandbox's refs. A branch on the host has no checkout
/// here, so none is `current` and none names a `worktreePath`; the default
/// leads the list already, as [`list_branches`] returns it default-first.
fn branch_ref_page(response: &BranchListResponse, request: &RefRequest<'_>) -> Value {
    // A host branch is a plain branch, not a remote-tracking ref, so a request
    // narrowed to remote refs has nothing to answer with.
    let matching: Vec<&BranchEntry> = if request.ref_kind == RefKind::Remote {
        Vec::new()
    } else {
        response
            .branches
            .iter()
            .filter(|branch| match request.query {
                Some(query) => branch.name.to_lowercase().contains(&query.to_lowercase()),
                None => true,
            })
            .collect()
    };

    let total_count = matching.len();
    let page: Vec<Value> = matching
        .into_iter()
        .skip(request.cursor)
        .take(request.limit)
        .map(|branch| {
            json!({
                "name": branch.name,
                "isRemote": false,
                "remoteName": Value::Null,
                "current": false,
                "isDefault": branch.is_default,
                "worktreePath": Value::Null,
            })
        })
        .collect();
    let next_cursor = match request.cursor.saturating_add(page.len()) {
        reached if reached < total_count => json!(reached),
        _ => Value::Null,
    };

    json!({
        "refs": page,
        "isRepo": true,
        "hasPrimaryRemote": true,
        "nextCursor": next_cursor,
        "totalCount": total_count,
    })
}

/// The primary repository's branch state, as the contract's status snapshot.
pub async fn status(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    pull_requests: &dyn BoundPullRequestSource,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<Value, WorkspaceAccessError> {
    let (status, bound) = read_status(conversations, sandboxes, pull_requests, viewer, cwd).await?;
    Ok(status_snapshot(&status, primary_pull_request(&bound)))
}

/// One `snapshot` event of the contract's status stream.
///
/// The local and remote halves are sent together because they are read
/// together: one `git status --porcelain=2 --branch` carries the branch, the
/// dirty paths and the ahead/behind counts, so splitting them would report a
/// remote half this backend never asked for separately.
fn status_snapshot(status: &GetGitStatusResponse, pr: Option<&BoundPullRequest>) -> Value {
    json!({
        "_tag": "snapshot",
        "local": local_status(status),
        "remote": remote_status(status, pr),
    })
}

/// The same branch state, flattened, as `vcs.refreshStatus` returns it.
///
/// The subscription and this method read one `git status`, so a refresh is the
/// same call the poll makes rather than a second notion of what the status is.
/// The client merges the stream's two halves into exactly this shape, which is
/// why the difference between them is only where the seam falls.
pub async fn status_result(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    pull_requests: &dyn BoundPullRequestSource,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<Value, WorkspaceAccessError> {
    let (status, bound) = read_status(conversations, sandboxes, pull_requests, viewer, cwd).await?;
    Ok(flat_status(&status, primary_pull_request(&bound)))
}

/// Both halves in one object, which is what the client's own reducer builds out
/// of the stream's `snapshot`.
fn flat_status(status: &GetGitStatusResponse, pr: Option<&BoundPullRequest>) -> Value {
    let mut flat = local_status(status);
    if let (Some(flat), Value::Object(remote)) = (flat.as_object_mut(), remote_status(status, pr)) {
        flat.extend(remote);
    }
    flat
}

/// The Sandbox's branch state and the pull requests the Task is bound to, read
/// together so a pull request that appears without the checkout changing still
/// makes the snapshot differ and reaches the poll's dedupe.
async fn read_status(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    pull_requests: &dyn BoundPullRequestSource,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<(GetGitStatusResponse, Vec<BoundPullRequest>), WorkspaceAccessError> {
    let task_id = workspace::addressed_conversation(conversations, viewer, cwd).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(workspace::sandbox_access)?;
    let status = client
        .get_git_status_for_task(Some(&task_id))
        .await
        .map_err(|status| {
            workspace::sandbox_access(classify_grpc_error(status, "get_git_status"))
        })?;
    let bound = pull_requests.bound_pull_requests(&task_id).await;
    Ok((status, bound))
}

/// The half of the status one `git status --porcelain=2 --branch` reads about
/// the checkout itself.
fn local_status(status: &GetGitStatusResponse) -> Value {
    let insertions: u32 = status
        .working_tree_files
        .iter()
        .map(|file| file.insertions)
        .sum();
    let deletions: u32 = status
        .working_tree_files
        .iter()
        .map(|file| file.deletions)
        .sum();
    json!({
        "isRepo": status.is_repo,
        "hasPrimaryRemote": status.has_primary_remote,
        "isDefaultRef": status.is_default_ref,
        "refName": if status.ref_name.is_empty() {
            Value::Null
        } else {
            json!(status.ref_name)
        },
        "hasWorkingTreeChanges": status.has_working_tree_changes,
        "workingTree": {
            "files": status
                .working_tree_files
                .iter()
                .map(|file| json!({
                    "path": file.path,
                    "insertions": file.insertions,
                    "deletions": file.deletions,
                }))
                .collect::<Vec<Value>>(),
            "insertions": insertions,
            "deletions": deletions,
        },
    })
}

/// The half about where the checkout stands against its upstream. Read from the
/// same command, so it is never reported without the local half beside it.
fn remote_status(status: &GetGitStatusResponse, pr: Option<&BoundPullRequest>) -> Value {
    json!({
        "hasUpstream": status.has_upstream,
        "aheadCount": status.ahead_count,
        "behindCount": status.behind_count,
        // Measured against the default branch, so Create PR stays enabled after
        // a push zeroes `aheadCount`.
        "aheadOfDefaultCount": status.ahead_of_default_count,
        // The pull request the Task is bound to, joined in from the Binding one
        // level up; `null` when it has none.
        "pr": pr.map_or(Value::Null, pull_request_json),
    })
}

/// One bound pull request, in the shape T3's `VcsStatusChangeRequest` wants.
fn pull_request_json(pr: &BoundPullRequest) -> Value {
    json!({
        "number": pr.number,
        "title": pr.title,
        "url": pr.url,
        "baseRef": pr.base_ref,
        "headRef": pr.head_ref,
        "state": t3_pr_state(&pr.state_kind),
        "isDraft": is_draft(&pr.state_kind),
    })
}

/// Why a linked pull request could not be summarized.
pub enum PullRequestSummaryError {
    /// No active binding names this reference, none the viewer may read, or
    /// no readable Task's binding resolved — a refresh that could not reach
    /// GitHub leaves the listing linking a pull request this cannot recover.
    /// One answer for the first two: telling them apart would say a Task exists.
    Unknown,
    Store(sqlx::Error),
}

/// The live fields of one linked pull request, as T3's `PullRequestSummary`.
///
/// Read from the readable Task's binding rather than from GitHub: every field
/// the summary declares is already persisted there, so the sidebar's per-thread
/// status check is a database read for a binding already fetched. A binding
/// whose status blob is still NULL — a freshly opened pull request no surface
/// has polled — costs one GitHub call to fill it, once, per cold binding. That
/// is also what bounds this method — it answers for a pull request some Task is
/// bound to, never for an arbitrary reference, which is `pullRequests.detail`'s
/// job and is not served.
pub async fn pull_request_summary(
    conversations: &dyn ConversationDao,
    pull_requests: &dyn BoundPullRequestSource,
    viewer: &ScopeContext,
    project_id: &str,
    repository: &str,
    number: u64,
) -> Result<Value, PullRequestSummaryError> {
    let task_ids = pull_requests.tasks_bound_to(repository, number).await;
    if task_ids.is_empty() {
        return Err(PullRequestSummaryError::Unknown);
    }
    let readable = conversations
        .readable_by_ids(viewer, &task_ids)
        .await
        .map_err(PullRequestSummaryError::Store)?;
    // Resolve only from a readable Task's binding: `bound_pull_requests`
    // refreshes an unfetched binding on the creator's token, so authorizing
    // first is what keeps a viewer from spending it on Tasks they cannot read.
    // Every binding on one reference recovers the same pull request, so the
    // first readable Task that still carries it answers for all of them.
    for conversation in &readable {
        if let Some(pr) = pull_requests
            .bound_pull_requests(&conversation.task_id)
            .await
            .into_iter()
            .find(|pr| pr.number == number && pr.repository.eq_ignore_ascii_case(repository))
        {
            return Ok(pull_request_summary_json(project_id, repository, &pr));
        }
    }
    Err(PullRequestSummaryError::Unknown)
}

fn pull_request_summary_json(project_id: &str, repository: &str, pr: &BoundPullRequest) -> Value {
    json!({
        "provider": "github",
        "projectId": project_id,
        // The reference the caller asked with, so the answer keys against the
        // request even where the binding recorded a different capitalization.
        "repository": repository,
        "number": pr.number,
        "title": pr.title,
        "url": pr.url,
        "state": t3_pr_state(&pr.state_kind),
        "isDraft": is_draft(&pr.state_kind),
        "headBranch": pr.head_ref,
        "baseBranch": pr.base_ref,
        "updatedAt": pr.status_read_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    })
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_proto::{GitRef, GitRepoRange, GitWorkingTreeFile};

    fn range(prefix: &str, base: &str, head: &str) -> GitRepoRange {
        GitRepoRange {
            prefix: prefix.into(),
            base_ref: base.into(),
            head_ref: head.into(),
        }
    }

    fn preview(ranges: Vec<GitRepoRange>) -> GetGitDiffPreviewResponse {
        GetGitDiffPreviewResponse {
            working_tree_diff: "dirty".into(),
            working_tree_truncated: false,
            branch_diff: "committed".into(),
            branch_truncated: true,
            ranges,
        }
    }

    fn git_ref(name: &str, remote: Option<&str>, current: bool, is_default: bool) -> GitRef {
        GitRef {
            name: name.into(),
            is_remote: remote.is_some(),
            remote_name: remote.unwrap_or_default().into(),
            current,
            is_default,
        }
    }

    fn listing(refs: Vec<GitRef>) -> ListGitRefsResponse {
        ListGitRefsResponse {
            is_repo: true,
            has_primary_remote: true,
            refs,
            truncated: false,
        }
    }

    fn request<'a>(cwd: &'a str, payload: &'a Value) -> RefRequest<'a> {
        RefRequest::from_payload(cwd, payload)
    }

    #[test]
    fn both_scopes_are_offered_even_when_one_has_no_patch() {
        let empty = GetGitDiffPreviewResponse::default();
        let sources = sources(&empty);
        let sources = sources.as_array().expect("an array of sources");

        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0]["kind"], "working-tree");
        assert_eq!(sources[1]["kind"], "branch-range");
        assert_eq!(sources[1]["title"], "Against base branch");
        assert_eq!(sources[1]["baseRef"], Value::Null);
    }

    #[test]
    fn a_single_repository_names_the_ref_each_scope_used() {
        let sources = sources(&preview(vec![range("repo", "origin/main", "feature")]));
        let branch = &sources.as_array().expect("an array")[1];

        assert_eq!(branch["baseRef"], "origin/main");
        assert_eq!(branch["headRef"], "feature");
        assert_eq!(branch["title"], "Against origin/main");
        assert_eq!(branch["truncated"], true);
    }

    /// One patch, several repositories: naming one repository's branch in the
    /// header would describe most of the patch wrongly.
    #[test]
    fn repositories_that_disagree_name_no_ref_at_all() {
        let sources = sources(&preview(vec![
            range("one", "origin/main", "feature"),
            range("two", "origin/trunk", "feature"),
        ]));
        let branch = &sources.as_array().expect("an array")[1];

        assert_eq!(branch["baseRef"], Value::Null);
        assert_eq!(branch["headRef"], "feature", "these two do agree on HEAD");
        assert_eq!(branch["title"], "Against base branch");
    }

    #[test]
    fn a_patch_is_identified_by_its_contents() {
        assert_eq!(diff_hash(""), diff_hash(""));
        assert_ne!(diff_hash("a"), diff_hash("b"));
        assert_eq!(diff_hash("a").len(), 64, "sha-256 as hex");
    }

    #[test]
    fn the_checked_out_branch_and_the_default_lead_the_first_page() {
        let refs = listing(vec![
            git_ref("aardvark", None, false, false),
            git_ref("origin/main", Some("origin"), false, true),
            git_ref("feature", None, true, false),
        ]);
        let payload = json!({ "includeMatchingRemoteRefs": true });
        let page = ref_page(&refs, &request("/moatless/tasks/t", &payload));

        let names: Vec<&str> = page["refs"]
            .as_array()
            .expect("an array")
            .iter()
            .map(|reference| reference["name"].as_str().expect("a name"))
            .collect();
        assert_eq!(names, vec!["feature", "origin/main", "aardvark"]);
        assert_eq!(page["totalCount"], 3);
        assert_eq!(page["nextCursor"], Value::Null);
    }

    #[test]
    fn a_query_matches_a_name_in_any_case() {
        let refs = listing(vec![
            git_ref("Feature/One", None, false, false),
            git_ref("main", None, false, true),
        ]);
        let payload = json!({ "query": "FEAT" });
        let page = ref_page(&refs, &request("/moatless/tasks/t", &payload));

        assert_eq!(page["totalCount"], 1);
        assert_eq!(page["refs"][0]["name"], "Feature/One");
    }

    #[test]
    fn a_page_reports_where_the_next_one_starts() {
        let refs = listing(vec![
            git_ref("a", None, false, false),
            git_ref("b", None, false, false),
            git_ref("c", None, false, false),
        ]);
        let first = json!({ "limit": 2 });
        let first = ref_page(&refs, &request("/moatless/tasks/t", &first));
        assert_eq!(first["nextCursor"], 2);
        assert_eq!(first["totalCount"], 3);

        let second = json!({ "limit": 2, "cursor": 2 });
        let second = ref_page(&refs, &request("/moatless/tasks/t", &second));
        assert_eq!(second["refs"].as_array().expect("an array").len(), 1);
        assert_eq!(second["nextCursor"], Value::Null);
    }

    #[test]
    fn a_branch_and_its_upstream_are_one_row_unless_both_were_asked_for() {
        let refs = listing(vec![
            git_ref("main", None, true, true),
            git_ref("origin/main", Some("origin"), false, true),
            git_ref("upstream/main", Some("upstream"), false, false),
        ]);

        let folded = ref_page(&refs, &request("/moatless/tasks/t", &json!({})));
        let names: Vec<&str> = folded["refs"]
            .as_array()
            .expect("an array")
            .iter()
            .map(|reference| reference["name"].as_str().expect("a name"))
            .collect();
        assert_eq!(
            names,
            vec!["main", "upstream/main"],
            "only origin's copy of a local branch folds away"
        );

        let both = json!({ "includeMatchingRemoteRefs": true });
        let both = ref_page(&refs, &request("/moatless/tasks/t", &both));
        assert_eq!(both["totalCount"], 3);
    }

    #[test]
    fn ref_kind_narrows_the_listing_to_one_namespace() {
        let refs = listing(vec![
            git_ref("main", None, true, true),
            git_ref("origin/other", Some("origin"), false, false),
        ]);

        let local = json!({ "refKind": "local", "includeMatchingRemoteRefs": true });
        assert_eq!(
            ref_page(&refs, &request("/moatless/tasks/t", &local))["totalCount"],
            1
        );
        let remote = json!({ "refKind": "remote", "includeMatchingRemoteRefs": true });
        let remote = ref_page(&refs, &request("/moatless/tasks/t", &remote));
        assert_eq!(remote["totalCount"], 1);
        assert_eq!(remote["refs"][0]["name"], "origin/other");
    }

    fn branches(names: &[(&str, bool)]) -> BranchListResponse {
        BranchListResponse {
            branches: names
                .iter()
                .map(|(name, is_default)| BranchEntry {
                    name: (*name).to_string(),
                    is_default: *is_default,
                })
                .collect(),
            default_branch: names
                .iter()
                .find(|(_, is_default)| *is_default)
                .map(|(name, _)| (*name).to_string()),
        }
    }

    #[test]
    fn host_branches_keep_their_default_first_order_and_carry_no_checkout() {
        let response = branches(&[("main", true), ("feature", false)]);
        let page = branch_ref_page(&response, &request("/moatless/workspaces/ws-1", &json!({})));

        assert_eq!(page["refs"][0]["name"], "main");
        assert_eq!(page["refs"][0]["isDefault"], true);
        assert_eq!(page["refs"][0]["current"], false);
        assert_eq!(page["refs"][0]["worktreePath"], Value::Null);
        assert_eq!(page["refs"][1]["name"], "feature");
        assert_eq!(page["totalCount"], 2);
        assert_eq!(page["isRepo"], true);
        assert_eq!(page["nextCursor"], Value::Null);
    }

    #[test]
    fn a_host_branch_query_matches_a_name_in_any_case() {
        let response = branches(&[("main", true), ("Feature/One", false)]);
        let payload = json!({ "query": "feat" });
        let page = branch_ref_page(&response, &request("/moatless/workspaces/ws-1", &payload));

        assert_eq!(page["totalCount"], 1);
        assert_eq!(page["refs"][0]["name"], "Feature/One");
    }

    #[test]
    fn a_host_branch_page_reports_where_the_next_one_starts() {
        let response = branches(&[("main", true), ("a", false), ("b", false)]);
        let first = ref_page_json(&response, json!({ "limit": 2 }));
        assert_eq!(first["nextCursor"], 2);
        assert_eq!(first["totalCount"], 3);

        let second = ref_page_json(&response, json!({ "limit": 2, "cursor": 2 }));
        assert_eq!(second["refs"].as_array().expect("an array").len(), 1);
        assert_eq!(second["nextCursor"], Value::Null);
    }

    /// A host branch is a plain branch, so a request narrowed to remote refs is
    /// answered with an empty page rather than every branch relabeled remote.
    #[test]
    fn host_branches_are_never_remote_refs() {
        let response = branches(&[("main", true)]);
        let payload = json!({ "refKind": "remote" });
        let page = branch_ref_page(&response, &request("/moatless/workspaces/ws-1", &payload));
        assert_eq!(page["totalCount"], 0);
    }

    fn ref_page_json(response: &BranchListResponse, payload: Value) -> Value {
        branch_ref_page(response, &request("/moatless/workspaces/ws-1", &payload))
    }

    // -- workspace_branches: the draft-thread branch listing ------------------

    use crate::dao::{Conversation, MockConversationDao};
    use crate::ports::{PullRequestError, ResolvedPullRequest};
    use crate::workspace_dao::MockT3WorkspaceDao;
    use async_trait::async_trait;
    use moatless_types::agent::AgentType;
    use std::sync::Mutex;

    /// A branch source that answers with a fixed list, or a fixed failure, and
    /// records the workspace it was asked on behalf of.
    struct StubBranches(Result<BranchListResponse, ()>, Mutex<Option<String>>);

    impl StubBranches {
        fn new(answer: Result<BranchListResponse, ()>) -> Self {
            Self(answer, Mutex::new(None))
        }

        fn asked_for_workspace(&self) -> Option<String> {
            self.1.lock().expect("uncontended").clone()
        }
    }

    #[async_trait]
    impl RepositoryBranchSource for StubBranches {
        async fn list_branches(
            &self,
            _viewer: &ScopeContext,
            _repository_id: &str,
            workspace_id: Option<&str>,
        ) -> Result<BranchListResponse, BranchListError> {
            *self.1.lock().expect("uncontended") = workspace_id.map(str::to_string);
            match &self.0 {
                Ok(response) => Ok(response.clone()),
                Err(()) => Err(BranchListError::Unavailable("connect your account".into())),
            }
        }
    }

    fn viewer() -> ScopeContext {
        ScopeContext {
            user_id: "user-1".to_string(),
            is_admin: false,
        }
    }

    /// A workspace DAO wired to answer with `primary`, the repository the
    /// picker would list. `None` is every reason there is nothing to list: the
    /// viewer cannot be shown the workspace, it is gone, or it declares no
    /// repository — the port answers them alike so none of them discloses which.
    fn workspaces(primary: Option<&str>) -> MockT3WorkspaceDao {
        let primary = primary.map(str::to_string);
        let mut dao = MockT3WorkspaceDao::new();
        dao.expect_primary_repository()
            .returning(move |_, _| Ok(primary.clone()));
        dao
    }

    fn project_root() -> RefRequest<'static> {
        RefRequest::from_payload("/moatless/workspaces/ws-1", &Value::Null)
    }

    /// The picker of a draft thread reaches the primary repository's branches,
    /// not a checkout that does not exist yet.
    #[tokio::test]
    async fn a_project_root_lists_the_primary_repositorys_branches() {
        let dao = workspaces(Some("repo-1"));
        let source = StubBranches::new(Ok(branches(&[("main", true), ("feature", false)])));
        let page = workspace_branches(
            &dao,
            &source,
            &viewer(),
            "/moatless/workspaces/ws-1",
            &project_root(),
        )
        .await
        .expect("a page of branches");

        assert_eq!(page["refs"][0]["name"], "main");
        assert_eq!(page["totalCount"], 2);
        assert_eq!(page["isRepo"], true);
    }

    /// The listing is made on behalf of the workspace the picker was opened in,
    /// so a repository only that workspace's pinned credential can reach still
    /// answers. A picker that asked as the viewer alone would show nothing for
    /// exactly the repositories the pin exists for.
    #[tokio::test]
    async fn the_picker_lists_on_behalf_of_the_workspace_it_was_opened_in() {
        let dao = workspaces(Some("repo-1"));
        let source = StubBranches::new(Ok(branches(&[("main", true)])));
        workspace_branches(
            &dao,
            &source,
            &viewer(),
            "/moatless/workspaces/ws-1",
            &project_root(),
        )
        .await
        .expect("a page of branches");

        assert_eq!(source.asked_for_workspace().as_deref(), Some("ws-1"));
    }

    /// A project root the viewer cannot be shown, and a workspace with no
    /// repository, are the same empty answer — never a disclosure that the
    /// workspace exists.
    #[tokio::test]
    async fn a_workspace_the_viewer_cannot_reach_lists_no_branches() {
        let dao = workspaces(None);
        let source = StubBranches::new(Ok(branches(&[("main", true)])));
        let result = workspace_branches(
            &dao,
            &source,
            &viewer(),
            "/moatless/workspaces/ws-1",
            &project_root(),
        )
        .await;
        assert!(matches!(result, Err(BranchListError::None)));
    }

    /// A repository whose git host would not answer surfaces its reason rather
    /// than an empty list, so the "connect your account" prompt can land.
    #[tokio::test]
    async fn an_unavailable_git_host_is_surfaced_not_swallowed() {
        let dao = workspaces(Some("repo-1"));
        let source = StubBranches::new(Err(()));
        let result = workspace_branches(
            &dao,
            &source,
            &viewer(),
            "/moatless/workspaces/ws-1",
            &project_root(),
        )
        .await;
        assert!(matches!(result, Err(BranchListError::Unavailable(_))));
    }

    #[test]
    fn a_status_snapshot_carries_both_halves_the_contract_wants() {
        let snapshot = status_snapshot(
            &GetGitStatusResponse {
                is_repo: true,
                ref_name: "feature".into(),
                is_default_ref: false,
                has_primary_remote: true,
                has_upstream: true,
                ahead_count: 2,
                behind_count: 1,
                ahead_of_default_count: 5,
                has_working_tree_changes: true,
                working_tree_files: vec![
                    GitWorkingTreeFile {
                        path: "repo/a.rs".into(),
                        insertions: 3,
                        deletions: 1,
                    },
                    GitWorkingTreeFile {
                        path: "repo/b.rs".into(),
                        insertions: 4,
                        deletions: 0,
                    },
                ],
            },
            None,
        );

        assert_eq!(snapshot["_tag"], "snapshot");
        assert_eq!(snapshot["local"]["refName"], "feature");
        assert_eq!(snapshot["local"]["hasWorkingTreeChanges"], true);
        assert_eq!(snapshot["local"]["workingTree"]["insertions"], 7);
        assert_eq!(snapshot["local"]["workingTree"]["deletions"], 1);
        assert_eq!(snapshot["remote"]["aheadCount"], 2);
        assert_eq!(snapshot["remote"]["aheadOfDefaultCount"], 5);
        assert_eq!(snapshot["remote"]["pr"], Value::Null);
    }

    fn bound_pr(state_kind: &str) -> BoundPullRequest {
        numbered_bound_pr(7, state_kind)
    }

    fn numbered_bound_pr(number: u64, state_kind: &str) -> BoundPullRequest {
        BoundPullRequest {
            number,
            repository: "acme/web".into(),
            title: "Add widgets".into(),
            url: format!("https://github.com/acme/web/pull/{number}"),
            base_ref: "main".into(),
            head_ref: "feature".into(),
            state_kind: state_kind.into(),
            status_read_at: chrono::Utc::now(),
        }
    }

    /// The singular `pr` field reports the newest binding a person can still
    /// act on, so a Task that opened a second pull request stops reporting
    /// whichever one the binding query happened to return first.
    #[test]
    fn the_primary_pull_request_prefers_a_live_binding_over_a_newer_settled_one() {
        let bound = vec![
            numbered_bound_pr(11, "open"),
            numbered_bound_pr(42, "merged"),
        ];
        assert_eq!(primary_pull_request(&bound).map(|pr| pr.number), Some(11));
    }

    #[test]
    fn the_primary_pull_request_takes_the_newest_of_several_live_bindings() {
        let bound = vec![
            numbered_bound_pr(11, "open"),
            numbered_bound_pr(42, "draft"),
        ];
        assert_eq!(primary_pull_request(&bound).map(|pr| pr.number), Some(42));
    }

    /// Every binding settled still reports one, so a merged pull request stays
    /// on the toolbar instead of the surface going blank.
    #[test]
    fn the_primary_pull_request_falls_back_to_the_newest_settled_binding() {
        let bound = vec![
            numbered_bound_pr(11, "closed"),
            numbered_bound_pr(42, "merged"),
        ];
        assert_eq!(primary_pull_request(&bound).map(|pr| pr.number), Some(42));
    }

    #[test]
    fn no_bindings_report_no_primary_pull_request() {
        assert!(primary_pull_request(&[]).is_none());
    }

    #[test]
    fn a_bound_pull_request_is_reported_with_its_branches() {
        let pr = bound_pr("open");
        let remote = remote_status(&GetGitStatusResponse::default(), Some(&pr));
        assert_eq!(remote["pr"]["number"], 7);
        assert_eq!(remote["pr"]["baseRef"], "main");
        assert_eq!(remote["pr"]["headRef"], "feature");
        assert_eq!(remote["pr"]["state"], "open");
        assert_eq!(remote["pr"]["isDraft"], false);
    }

    /// T3's `VcsStatusChangeRequestState` has no `draft`, so a draft binding
    /// reports as `open` and says so in `isDraft`.
    #[test]
    fn a_draft_binding_reports_as_open() {
        let pr = bound_pr("draft");
        let remote = remote_status(&GetGitStatusResponse::default(), Some(&pr));
        assert_eq!(remote["pr"]["state"], "open");
        assert_eq!(remote["pr"]["isDraft"], true);
    }

    #[test]
    fn merged_and_closed_bindings_keep_their_state() {
        assert_eq!(
            remote_status(&GetGitStatusResponse::default(), Some(&bound_pr("merged")))["pr"]["state"],
            "merged"
        );
        assert_eq!(
            remote_status(&GetGitStatusResponse::default(), Some(&bound_pr("closed")))["pr"]["state"],
            "closed"
        );
    }

    #[test]
    fn a_task_with_no_binding_reports_a_null_pull_request() {
        let remote = remote_status(&GetGitStatusResponse::default(), None);
        assert_eq!(remote["pr"], Value::Null);
    }

    /// The contract requires `refName` to be a non-empty string or null, and a
    /// detached HEAD has no name to give.
    #[test]
    fn a_detached_head_reports_no_branch_rather_than_an_empty_one() {
        let snapshot = status_snapshot(
            &GetGitStatusResponse {
                is_repo: true,
                ..GetGitStatusResponse::default()
            },
            None,
        );
        assert_eq!(snapshot["local"]["refName"], Value::Null);
        assert_eq!(snapshot["local"]["workingTree"]["files"], json!([]));
    }

    /// The four states a person cannot act on, against the two this deployment
    /// owns. Getting the second group wrong hides a broken backend behind a
    /// branch picker that merely looks empty.
    #[test]
    fn only_a_sandbox_nobody_can_ask_makes_an_empty_ref_list_honest() {
        assert!(refs_are_quiet(&WorkspaceAccessError::NoWorktree));
        assert!(refs_are_quiet(&WorkspaceAccessError::Unaddressable));
        assert!(refs_are_quiet(&WorkspaceAccessError::NotRunning(
            SandboxError::NotReady("t-1".into())
        )));
        assert!(refs_are_quiet(&WorkspaceAccessError::Sandbox(
            SandboxError::Unsupported {
                operation: "list_git_refs".into(),
                source: Box::new(tonic::Status::unimplemented("unknown method")),
            }
        )));

        assert!(!refs_are_quiet(&WorkspaceAccessError::Sandbox(
            SandboxError::Grpc {
                operation: "list_git_refs".into(),
                source: Box::new(tonic::Status::internal("boom")),
            }
        )));
        assert!(!refs_are_quiet(&WorkspaceAccessError::Database(
            sqlx::Error::RowNotFound
        )));
    }

    /// `vcs.refreshStatus` returns what the stream's two halves merge into, so
    /// a refresh and a poll cannot disagree about the same repository.
    #[test]
    fn a_refresh_returns_the_halves_of_a_snapshot_merged_into_one_object() {
        let status = GetGitStatusResponse {
            is_repo: true,
            ref_name: "feature".into(),
            has_upstream: true,
            ahead_count: 2,
            ..GetGitStatusResponse::default()
        };
        let snapshot = status_snapshot(&status, None);
        let flat = flat_status(&status, None);

        for (key, value) in snapshot["local"]
            .as_object()
            .into_iter()
            .chain(snapshot["remote"].as_object())
            .flatten()
        {
            assert_eq!(&flat[key], value, "{key} differs from the snapshot's");
        }
        assert!(flat.get("_tag").is_none(), "a result, not a stream event");
    }

    // -- pullRequests.summary: the linked thread's live fields ----------------

    /// A binding source that answers with fixed Task-to-pull-request pairs and
    /// records every Task id `bound_pull_requests` was asked to resolve, so a
    /// test can prove resolution ran only after authorization.
    struct StubBoundPullRequests {
        bound: Vec<(String, BoundPullRequest)>,
        resolved: std::sync::Mutex<Vec<String>>,
    }

    impl StubBoundPullRequests {
        fn new(bound: Vec<(String, BoundPullRequest)>) -> Self {
            Self {
                bound,
                resolved: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn resolved(&self) -> Vec<String> {
            self.resolved.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl BoundPullRequestSource for StubBoundPullRequests {
        async fn bound_pull_requests(&self, task_id: &str) -> Vec<BoundPullRequest> {
            self.resolved.lock().unwrap().push(task_id.to_string());
            self.bound
                .iter()
                .filter(|(bound_task_id, _)| bound_task_id == task_id)
                .map(|(_, pr)| pr.clone())
                .collect()
        }

        async fn tasks_bound_to(&self, _repository: &str, _number: u64) -> Vec<String> {
            self.bound
                .iter()
                .map(|(task_id, _)| task_id.clone())
                .collect()
        }

        async fn resolve_pull_request(
            &self,
            _viewer: &ScopeContext,
            _repository_id: &str,
            _reference: &str,
        ) -> Result<ResolvedPullRequest, PullRequestError> {
            unreachable!("the summary resolves no reference")
        }

        async fn open_pull_request_for_head(
            &self,
            _viewer: &ScopeContext,
            _repository_id: &str,
            _head: &str,
        ) -> Option<ResolvedPullRequest> {
            unreachable!("the summary looks up no head")
        }
    }

    /// The summary path reads nothing off the row but its id — it is an access
    /// answer, not a projection — so the rest is left at its empty value.
    fn a_conversation(task_id: &str) -> Conversation {
        Conversation {
            task_id: task_id.to_string(),
            workspace_id: Some("ws-1".to_string()),
            name: None,
            branch: None,
            agent_type: AgentType(AgentType::CLAUDE_CODE.to_string()),
            created_at: "2026-07-01T09:00:00.000Z".to_string(),
            created_at_db: None,
            updated_at: None,
            closed_at: None,
            visibility: moatless_types::task::Visibility::Private,
            owner_name: None,
            turn_number: None,
            turn_status: None,
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state_raw: None,
            sandbox_stop_reason_raw: None,
            sandbox_started_at: None,
            sandbox_updated_at: None,
            running_commands: 0,
            latest_user_message_at: None,
            assistant_model: None,
            requested_model: None,
            pending_question: false,
            pending_plan: false,
            settled_override: None,
            settled_at: None,
            snoozed_until: None,
            snoozed_at: None,
            pull_requests: None,
        }
    }

    fn readable_conversations(readable: bool) -> MockConversationDao {
        let mut dao = MockConversationDao::new();
        dao.expect_readable_by_ids().returning(move |_, task_ids| {
            Ok(if readable {
                task_ids.iter().map(|id| a_conversation(id)).collect()
            } else {
                Vec::new()
            })
        });
        dao
    }

    async fn summary(state_kind: &str, readable: bool) -> Result<Value, PullRequestSummaryError> {
        let bound = StubBoundPullRequests::new(vec![("task-a".to_string(), bound_pr(state_kind))]);
        pull_request_summary(
            &readable_conversations(readable),
            &bound,
            &viewer(),
            "ws-1",
            "acme/web",
            7,
        )
        .await
    }

    /// Every field of T3's `PullRequestSummary` comes off the binding, so the
    /// sidebar's per-thread status check reaches no git host.
    #[tokio::test]
    async fn a_linked_pull_request_is_summarized_from_its_binding() {
        let summary = summary("open", true)
            .await
            .ok()
            .expect("a readable task answers");
        assert_eq!(summary["provider"], "github");
        assert_eq!(summary["projectId"], "ws-1");
        assert_eq!(summary["repository"], "acme/web");
        assert_eq!(summary["number"], 7);
        assert_eq!(summary["title"], "Add widgets");
        assert_eq!(summary["url"], "https://github.com/acme/web/pull/7");
        assert_eq!(summary["state"], "open");
        assert_eq!(summary["isDraft"], false);
        assert_eq!(summary["headBranch"], "feature");
        assert_eq!(summary["baseBranch"], "main");
        assert!(
            summary["updatedAt"]
                .as_str()
                .is_some_and(|at| at.ends_with('Z'))
        );
    }

    /// The sidebar and the composer's pull-request menu tell a draft from an
    /// open pull request by `isDraft` alone, since both report `state` as
    /// `open`.
    #[tokio::test]
    async fn a_draft_pull_request_is_summarized_as_a_draft() {
        let summary = summary("draft", true)
            .await
            .ok()
            .expect("a readable task answers");
        assert_eq!(summary["state"], "open");
        assert_eq!(summary["isDraft"], true);
    }

    /// The bound Task is the access rule: a viewer who cannot read it is told
    /// the same thing as one asking about a pull request nothing is bound to.
    #[tokio::test]
    async fn a_pull_request_on_an_unreadable_task_is_not_summarized() {
        assert!(matches!(
            summary("open", false).await,
            Err(PullRequestSummaryError::Unknown)
        ));
    }

    #[tokio::test]
    async fn a_pull_request_nothing_is_bound_to_is_not_summarized() {
        let result = pull_request_summary(
            &readable_conversations(true),
            &StubBoundPullRequests::new(Vec::new()),
            &viewer(),
            "ws-1",
            "acme/web",
            7,
        )
        .await;
        assert!(matches!(result, Err(PullRequestSummaryError::Unknown)));
    }

    /// Resolving a binding refreshes it on the Task creator's GitHub token, so
    /// it must not run for a Task the viewer cannot read: otherwise naming an
    /// arbitrary reference spends another user's token and turns the `Unknown`
    /// answer into a slow one that says a Task exists.
    #[tokio::test]
    async fn an_unreadable_task_is_never_resolved() {
        let bound = StubBoundPullRequests::new(vec![("task-a".to_string(), bound_pr("open"))]);
        let result = pull_request_summary(
            &readable_conversations(false),
            &bound,
            &viewer(),
            "ws-1",
            "acme/web",
            7,
        )
        .await;
        assert!(matches!(result, Err(PullRequestSummaryError::Unknown)));
        assert!(
            bound.resolved().is_empty(),
            "resolved a binding before authorizing the viewer: {:?}",
            bound.resolved()
        );
    }
}
