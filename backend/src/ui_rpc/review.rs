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
//! the end of one turn, diffed against the turn before. Moatless records no
//! such snapshot, so a conversation's `checkpoints` are empty and the panel
//! offers the two git scopes alone. Deriving a turn diff from the transcript
//! would put a revert control and a per-turn patch in front of a person that
//! nothing behind them can honour.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::dao::ConversationDao;
use super::workspace::{self, WorkspaceAccessError};
use crate::sandbox::access::SandboxConnector;
use crate::sandbox::client::classify_grpc_error;
use crate::sandbox::error::SandboxError;
use moatless_backend_core::scope::ScopeContext;
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
    let preview = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(WorkspaceAccessError::NotRunning)?
        .get_git_diff_preview(base_ref.map(str::to_string), ignore_whitespace)
        .await
        .map_err(|status| {
            WorkspaceAccessError::Sandbox(classify_grpc_error(status, "get_git_diff_preview"))
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
    let listing = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(WorkspaceAccessError::NotRunning)?
        .list_git_refs()
        .await
        .map_err(|status| {
            WorkspaceAccessError::Sandbox(classify_grpc_error(status, "list_git_refs"))
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

/// The primary repository's branch state, as the contract's status snapshot.
pub async fn status(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<Value, WorkspaceAccessError> {
    let status = read_status(conversations, sandboxes, viewer, cwd).await?;
    Ok(status_snapshot(&status))
}

/// One `snapshot` event of the contract's status stream.
///
/// The local and remote halves are sent together because they are read
/// together: one `git status --porcelain=2 --branch` carries the branch, the
/// dirty paths and the ahead/behind counts, so splitting them would report a
/// remote half this backend never asked for separately.
fn status_snapshot(status: &GetGitStatusResponse) -> Value {
    json!({
        "_tag": "snapshot",
        "local": local_status(status),
        "remote": remote_status(status),
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
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<Value, WorkspaceAccessError> {
    let status = read_status(conversations, sandboxes, viewer, cwd).await?;
    Ok(flat_status(&status))
}

/// Both halves in one object, which is what the client's own reducer builds out
/// of the stream's `snapshot`.
fn flat_status(status: &GetGitStatusResponse) -> Value {
    let mut flat = local_status(status);
    if let (Some(flat), Value::Object(remote)) = (flat.as_object_mut(), remote_status(status)) {
        flat.extend(remote);
    }
    flat
}

async fn read_status(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<GetGitStatusResponse, WorkspaceAccessError> {
    let task_id = workspace::addressed_conversation(conversations, viewer, cwd).await?;
    sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(WorkspaceAccessError::NotRunning)?
        .get_git_status()
        .await
        .map_err(|status| {
            WorkspaceAccessError::Sandbox(classify_grpc_error(status, "get_git_status"))
        })
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
fn remote_status(status: &GetGitStatusResponse) -> Value {
    json!({
        "hasUpstream": status.has_upstream,
        "aheadCount": status.ahead_count,
        "behindCount": status.behind_count,
        // Moatless opens pull requests through its own adapters rather than
        // from a checkout, so a Sandbox knows of none.
        "pr": Value::Null,
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

    #[test]
    fn a_status_snapshot_carries_both_halves_the_contract_wants() {
        let snapshot = status_snapshot(&GetGitStatusResponse {
            is_repo: true,
            ref_name: "feature".into(),
            is_default_ref: false,
            has_primary_remote: true,
            has_upstream: true,
            ahead_count: 2,
            behind_count: 1,
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
        });

        assert_eq!(snapshot["_tag"], "snapshot");
        assert_eq!(snapshot["local"]["refName"], "feature");
        assert_eq!(snapshot["local"]["hasWorkingTreeChanges"], true);
        assert_eq!(snapshot["local"]["workingTree"]["insertions"], 7);
        assert_eq!(snapshot["local"]["workingTree"]["deletions"], 1);
        assert_eq!(snapshot["remote"]["aheadCount"], 2);
        assert_eq!(snapshot["remote"]["pr"], Value::Null);
    }

    /// The contract requires `refName` to be a non-empty string or null, and a
    /// detached HEAD has no name to give.
    #[test]
    fn a_detached_head_reports_no_branch_rather_than_an_empty_one() {
        let snapshot = status_snapshot(&GetGitStatusResponse {
            is_repo: true,
            ..GetGitStatusResponse::default()
        });
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
                source: tonic::Status::unimplemented("unknown method"),
            }
        )));

        assert!(!refs_are_quiet(&WorkspaceAccessError::Sandbox(
            SandboxError::Grpc {
                operation: "list_git_refs".into(),
                source: tonic::Status::internal("boom"),
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
        let snapshot = status_snapshot(&status);
        let flat = flat_status(&status);

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
}
