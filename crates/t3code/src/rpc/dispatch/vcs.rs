//! Methods that read a checkout, a diff or a pull request.

use super::routing::Request;
use crate::authentication::viewer_scope;
use crate::git_actions;
use crate::nominal_path;
use crate::review;
use crate::rpc::frames::branch_list_exit;
use crate::rpc::frames::branch_switch_result;
use crate::rpc::frames::git_action_failure_exit;
use crate::rpc::frames::pull_request_failure_exit;
use crate::rpc::frames::pull_request_ref;
use crate::rpc::frames::pull_request_summary_failure_exit;
use crate::rpc::frames::review_failure_exit;
use crate::rpc::frames::success_exit;
use crate::rpc::frames::unsupported_exit;
use crate::snapshots::workspace_cwd;
use crate::workspace::WorkspaceAccessError;
use serde_json::Value;
use serde_json::json;
use tracing::error;

/// A draft thread has no checkout to switch: its branch is recorded on the draft
/// and applied when the Task is created, so the switch is answered as done,
/// echoing the ref the client should store. A thread that already has a Sandbox
/// cannot change branch — that is fixed at creation — so the switch there falls
/// through to unsupported.
pub(super) fn vcs_switch_ref(request: Request) -> Vec<Value> {
    match branch_switch_result(&request.payload) {
        Some(result) => vec![success_exit(&request.id, result)],
        None => vec![unsupported_exit(&request.id, &request.tag)],
    }
}

pub(super) async fn vcs_list_refs(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(&payload) else {
        return vec![success_exit(&id, review::no_refs())];
    };
    let refs = review::RefRequest::from_payload(&cwd, &payload);
    // A draft thread has no Sandbox yet and addresses its Workspace by the
    // project root. Its branches are the primary repository's, read from the git
    // host — the same list the Moatless branch picker shows before a Task runs.
    if nominal_path::is_project_root(&cwd) {
        return match review::workspace_branches(
            state.reads.workspaces(),
            state.branches.as_ref(),
            &viewer_scope(&identity),
            &cwd,
            &refs,
        )
        .await
        {
            Ok(refs) => vec![success_exit(&id, refs)],
            Err(error) => vec![branch_list_exit(&id, &cwd, error, &identity)],
        };
    }
    match review::list_refs(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &viewer_scope(&identity),
        &refs,
    )
    .await
    {
        Ok(refs) => vec![success_exit(&id, refs)],
        Err(error) if review::refs_are_quiet(&error) => vec![success_exit(&id, review::no_refs())],
        Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, &identity)],
    }
}

pub(super) async fn vcs_refresh_status(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(&payload) else {
        // No cwd is why this failed, so there is none to report.
        return vec![review_failure_exit(
            &id,
            &tag,
            "",
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match review::status_result(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        state.pull_requests.as_ref(),
        &viewer_scope(&identity),
        &cwd,
    )
    .await
    {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, &identity)],
    }
}

pub(super) async fn review_get_diff_preview(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(&payload) else {
        // No cwd is why this failed, so there is none to report.
        return vec![review_failure_exit(
            &id,
            &tag,
            "",
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match review::diff_preview(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &viewer_scope(&identity),
        &cwd,
        payload
            .get("baseRef")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|base| !base.is_empty()),
        payload
            .get("ignoreWhitespace")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
    .await
    {
        Ok(preview) => vec![success_exit(&id, preview)],
        Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, &identity)],
    }
}

/// The one method of the `pullRequests.*` group this backend serves. The rest of
/// the group reads a git host and is refused, which is why
/// `capabilities.pullRequests` stays absent: the listing route, the right-panel
/// surface and the review actions all gate on it, and none of them can be served
/// from a binding.
pub(super) async fn pull_requests_summary(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some((project_id, repository, number)) = pull_request_ref(&payload) else {
        return vec![pull_request_summary_failure_exit(
            &id,
            "A pull request reference needs a projectId, repository and number.",
        )];
    };
    match review::pull_request_summary(
        state.reads.conversations(),
        state.pull_requests.as_ref(),
        &viewer_scope(&identity),
        &project_id,
        &repository,
        number,
    )
    .await
    {
        Ok(summary) => vec![success_exit(&id, summary)],
        Err(review::PullRequestSummaryError::Unknown) => {
            vec![pull_request_summary_failure_exit(
                &id,
                "No pull request is linked to a task you can read.",
            )]
        }
        Err(review::PullRequestSummaryError::Store(error)) => {
            error!(%error, user_id = %identity.user_id, "T3 pull-request summary read failed");
            vec![pull_request_summary_failure_exit(
                &id,
                "The pull request could not be read.",
            )]
        }
    }
}

pub(super) async fn git_resolve_pull_request(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    match git_actions::resolve_pull_request(
        state.reads.workspaces(),
        state.reads.conversations(),
        state.pull_requests.as_ref(),
        &viewer_scope(&identity),
        &payload,
    )
    .await
    {
        Ok(result) => vec![success_exit(&id, result)],
        Err(error) => vec![pull_request_failure_exit(&id, &tag, &error, &identity)],
    }
}

pub(super) async fn git_prepare_pull_request_thread(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    match git_actions::prepare_pull_request_thread(
        state.reads.workspaces(),
        state.reads.conversations(),
        state.pull_requests.as_ref(),
        &viewer_scope(&identity),
        &payload,
    )
    .await
    {
        Ok(git_actions::PreparedThread::Local(result)) => vec![success_exit(&id, result)],
        // Worktree mode is refused by name, not by falling through to the
        // catch-all: this surface prepares a branch, never a worktree on disk.
        Ok(git_actions::PreparedThread::WorktreeUnsupported) => vec![unsupported_exit(&id, &tag)],
        Err(error) => vec![pull_request_failure_exit(&id, &tag, &error, &identity)],
    }
}

/// The action is not run here or in the Sandbox: the button press becomes a
/// message asking the Task's agent to do the git work. The client's contract is
/// a stream, so it is answered with one `action_finished` chunk and a terminal
/// success, the same shape a completed action produced.
pub(super) async fn git_run_stacked_action(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    match git_actions::run_stacked_action(
        state.reads.conversations(),
        state.commands.as_ref(),
        &viewer_scope(&identity),
        &payload,
    )
    .await
    {
        Ok(sent) => vec![
            json!({
                "_tag": "Chunk",
                "requestId": id.wire(),
                "values": [git_actions::asked_agent_event(&sent)],
            }),
            success_exit(&id, Value::Null),
        ],
        Err(error) => vec![git_action_failure_exit(&id, &error, &identity)],
    }
}
