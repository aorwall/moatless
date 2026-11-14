//! Methods that read a checkout, a diff or a pull request.

use crate::authentication::viewer_scope;
use crate::git_actions;
use crate::git_actions::GitPreparePullRequestThreadInput;
use crate::git_actions::GitPullRequestRefInput;
use crate::git_actions::GitRunStackedActionInput;
use crate::nominal_path;
use crate::oauth::Scope;
use crate::pull_request_conversation;
use crate::pull_request_conversation::PullRequestDiffFileContentsInput;
use crate::pull_request_conversation::PullRequestThreadCommentsInput;
use crate::pull_request_list;
use crate::pull_request_list::PullRequestListInput;
use crate::pull_request_review;
use crate::pull_request_review::PullRequestRef;
use crate::pull_request_writes;
use crate::pull_request_writes::{WriteInput, WriteMethod};
use crate::review;
use crate::review::VcsListRefsInput;
use crate::rpc::envelope::RequestId;
use crate::rpc::envelope::success_exit;
use crate::rpc::envelope::typed_failure_exit;
use crate::rpc::errors::branch_list_exit;
use crate::rpc::errors::git_action_failure_exit;
use crate::rpc::errors::pull_request_failure_exit;
use crate::rpc::errors::pull_request_review_error;
use crate::rpc::errors::pull_request_summary_failure_exit;
use crate::rpc::errors::review_failure_exit;
use crate::rpc::errors::unsupported_exit;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use crate::snapshots::workspace_cwd;
use crate::workspace::WorkspaceAccessError;
use moatless_types::pull_request_conversation::PullRequestWrite;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Value;
use serde_json::json;
use tracing::error;

pub(super) fn vcs_methods() -> MethodTable {
    MethodTable::new()
        .detached(
            "git.preparePullRequestThread",
            Scope::OrchestrationOperate,
            git_prepare_pull_request_thread,
        )
        .detached(
            "git.resolvePullRequest",
            Scope::OrchestrationOperate,
            git_resolve_pull_request,
        )
        .session(
            "git.runStackedAction",
            Scope::OrchestrationOperate,
            git_run_stacked_action,
        )
        .detached(
            "pullRequests.activity",
            Scope::OrchestrationRead,
            pull_requests_activity,
        )
        .detached(
            "pullRequests.changeBreakdown",
            Scope::OrchestrationRead,
            pull_requests_change_breakdown,
        )
        .detached(
            "pullRequests.checks",
            Scope::OrchestrationRead,
            pull_requests_checks,
        )
        .detached(
            "pullRequests.comment",
            Scope::OrchestrationOperate,
            pull_requests_comment,
        )
        .detached(
            "pullRequests.detail",
            Scope::OrchestrationRead,
            pull_requests_detail,
        )
        .detached(
            "pullRequests.diffFileContents",
            Scope::OrchestrationRead,
            pull_requests_diff_file_contents,
        )
        .detached(
            "pullRequests.generateReviewGuide",
            Scope::OrchestrationOperate,
            pull_requests_generate_review_guide,
        )
        .detached(
            "pullRequests.linkedThreads",
            Scope::OrchestrationRead,
            pull_requests_linked_threads,
        )
        .detached(
            "pullRequests.invalidate",
            Scope::OrchestrationRead,
            pull_requests_invalidate,
        )
        .detached(
            "pullRequests.list",
            Scope::OrchestrationRead,
            pull_requests_list,
        )
        .detached(
            "pullRequests.labelCandidates",
            Scope::OrchestrationRead,
            pull_requests_label_candidates,
        )
        .detached(
            "pullRequests.listStats",
            Scope::OrchestrationRead,
            pull_requests_list_stats,
        )
        .detached(
            "pullRequests.preview",
            Scope::OrchestrationRead,
            pull_requests_preview,
        )
        .detached(
            "pullRequests.replyToThread",
            Scope::OrchestrationOperate,
            pull_requests_reply_to_thread,
        )
        .detached(
            "pullRequests.requestReviewers",
            Scope::OrchestrationOperate,
            pull_requests_request_reviewers,
        )
        .detached(
            "pullRequests.reviewGuide",
            Scope::OrchestrationRead,
            pull_requests_review_guide,
        )
        .detached(
            "pullRequests.reviewerCandidates",
            Scope::OrchestrationRead,
            pull_requests_reviewer_candidates,
        )
        .detached(
            "pullRequests.runAction",
            Scope::OrchestrationOperate,
            pull_requests_run_action,
        )
        .detached(
            "pullRequests.setLabels",
            Scope::OrchestrationOperate,
            pull_requests_set_labels,
        )
        .detached(
            "pullRequests.setReaction",
            Scope::OrchestrationOperate,
            pull_requests_set_reaction,
        )
        .detached(
            "pullRequests.setThreadResolution",
            Scope::OrchestrationOperate,
            pull_requests_set_thread_resolution,
        )
        .detached(
            "pullRequests.stack",
            Scope::OrchestrationRead,
            pull_requests_stack,
        )
        .detached(
            "pullRequests.submitReview",
            Scope::OrchestrationOperate,
            pull_requests_submit_review,
        )
        .detached(
            "pullRequests.summary",
            Scope::OrchestrationRead,
            pull_requests_summary,
        )
        .detached(
            "pullRequests.threadComments",
            Scope::OrchestrationRead,
            pull_requests_thread_comments,
        )
        .detached(
            "pullRequests.update",
            Scope::OrchestrationOperate,
            pull_requests_update,
        )
        .detached(
            "pullRequests.updateComment",
            Scope::OrchestrationOperate,
            pull_requests_update_comment,
        )
        .detached(
            "review.getDiffFileContents",
            Scope::ReviewWrite,
            review_get_diff_file_contents,
        )
        .detached(
            "review.getDiffPreview",
            Scope::ReviewWrite,
            review_get_diff_preview,
        )
        .declared("vcs.createRef", Scope::OrchestrationOperate)
        .declared("vcs.createWorktree", Scope::OrchestrationOperate)
        .declared("vcs.init", Scope::OrchestrationOperate)
        .detached("vcs.listRefs", Scope::OrchestrationRead, vcs_list_refs)
        .declared("vcs.pull", Scope::OrchestrationOperate)
        .detached(
            "vcs.refreshStatus",
            Scope::OrchestrationRead,
            vcs_refresh_status,
        )
        .declared("vcs.removeWorktree", Scope::OrchestrationOperate)
        .session("vcs.switchRef", Scope::OrchestrationOperate, vcs_switch_ref)
}

/// The contract's `VcsSwitchRefInput`. Optional here because a request
/// without either field is answered as unsupported, not refused by the decode.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct VcsSwitchRefInput {
    cwd: Option<String>,
    ref_name: Option<String>,
}

/// The contract's `VcsStatusInput`. Optional here because a request without a
/// `cwd` is answered with the typed refusal `review_failure_exit` gives.
#[derive(Deserialize)]
pub(super) struct VcsStatusInput {
    cwd: Option<String>,
}

/// The contract's `ReviewDiffPreviewInput`, as far as it is read; `cwd` is
/// optional for the reason [`VcsStatusInput`]'s is.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewDiffPreviewInput {
    cwd: Option<String>,
    base_ref: Option<String>,
    ignore_whitespace: Option<bool>,
}

/// A draft thread has no checkout, so the switch answers done, echoing back the
/// ref to store; a thread with a Sandbox can't change branch (fixed at
/// creation) and falls through to unsupported.
pub(super) async fn vcs_switch_ref(request: Request<VcsSwitchRefInput>) -> Vec<Value> {
    match branch_switch_result(&request.payload) {
        Some(result) => vec![success_exit(&request.id, result)],
        None => vec![unsupported_exit(&request.id, request.tag)],
    }
}

pub(super) async fn vcs_list_refs(request: Request<VcsListRefsInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(payload.cwd.as_deref()) else {
        return vec![success_exit(&id, review::no_refs())];
    };
    let refs = review::RefRequest::from_input(&cwd, &payload);
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
        Err(error) => vec![review_failure_exit(&id, tag, &cwd, &error, &identity)],
    }
}

pub(super) async fn vcs_refresh_status(request: Request<VcsStatusInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(payload.cwd.as_deref()) else {
        // No cwd is why this failed, so there is none to report.
        return vec![review_failure_exit(
            &id,
            tag,
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
        Err(error) => vec![review_failure_exit(&id, tag, &cwd, &error, &identity)],
    }
}

pub(super) async fn review_get_diff_preview(
    request: Request<ReviewDiffPreviewInput>,
) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(payload.cwd.as_deref()) else {
        // No cwd is why this failed, so there is none to report.
        return vec![review_failure_exit(
            &id,
            tag,
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
            .base_ref
            .as_deref()
            .map(str::trim)
            .filter(|base| !base.is_empty()),
        payload.ignore_whitespace.unwrap_or(false),
    )
    .await
    {
        Ok(preview) => vec![success_exit(&id, preview)],
        Err(error) => vec![review_failure_exit(&id, tag, &cwd, &error, &identity)],
    }
}

/// The contract's `ReviewDiffFileContentsInput`; `cwd` is optional for the
/// reason [`VcsStatusInput`]'s is.
#[derive(Deserialize)]
pub(super) struct ReviewDiffFileContentsInput {
    cwd: Option<String>,
    #[serde(flatten)]
    file: review::DiffFileContentsInput,
}

pub(super) async fn review_get_diff_file_contents(
    request: Request<ReviewDiffFileContentsInput>,
) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(payload.cwd.as_deref()) else {
        // No cwd is why this failed, so there is none to report.
        return vec![review_failure_exit(
            &id,
            tag,
            "",
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match review::diff_file_contents(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &viewer_scope(&identity),
        &cwd,
        &payload.file,
    )
    .await
    {
        Ok(contents) => vec![success_exit(&id, contents)],
        Err(error) => vec![review_failure_exit(&id, tag, &cwd, &error, &identity)],
    }
}

/// Every readable project's pull requests, read from GitHub.
pub(super) async fn pull_requests_list(request: Request<PullRequestListInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let ports = pull_request_review::review_ports(&state);
    match pull_request_list::list(&ports, &viewer_scope(&identity), &payload).await {
        Ok(value) => vec![success_exit(&id, value)],
        Err(error) => vec![typed_failure_exit(
            &id,
            pull_request_review_error(tag, &error, &identity),
        )],
    }
}

/// No stats: `pullRequests.list` already carries each row's line counts, and
/// the page keeps those for a row absent here.
pub(super) async fn pull_requests_list_stats(request: Request<IgnoredAny>) -> Vec<Value> {
    vec![success_exit(&request.id, json!({ "stats": [] }))]
}

/// Nothing to forget: no listing, detail or diff is cached here.
pub(super) async fn pull_requests_invalidate(request: Request<IgnoredAny>) -> Vec<Value> {
    vec![success_exit(&request.id, Value::Null)]
}

/// Which `pullRequests.*` read a request is.
enum PullRequestRead {
    Detail,
    ReviewGuide,
    GenerateReviewGuide,
    Activity,
    Preview,
    LinkedThreads,
    ThreadComments {
        thread_id: Option<String>,
        cursor: Option<String>,
    },
    DiffFileContents(PullRequestDiffFileContentsInput),
    Checks,
    ChangeBreakdown,
    LabelCandidates,
    ReviewerCandidates(Option<String>),
    Write(WriteMethod, Box<WriteInput>),
}

/// The read a request for `reference` alone is.
fn plain(
    read: PullRequestRead,
) -> impl FnOnce(PullRequestRef) -> (PullRequestRef, PullRequestRead) {
    move |reference| (reference, read)
}

pub(super) async fn pull_requests_activity(request: Request<PullRequestRef>) -> Vec<Value> {
    pull_request_read(request, plain(PullRequestRead::Activity)).await
}

pub(super) async fn pull_requests_preview(request: Request<PullRequestRef>) -> Vec<Value> {
    pull_request_read(request, plain(PullRequestRead::Preview)).await
}

pub(super) async fn pull_requests_linked_threads(request: Request<PullRequestRef>) -> Vec<Value> {
    pull_request_read(request, plain(PullRequestRead::LinkedThreads)).await
}

pub(super) async fn pull_requests_thread_comments(
    request: Request<PullRequestThreadCommentsInput>,
) -> Vec<Value> {
    pull_request_read(request, |input| {
        (
            input.reference,
            PullRequestRead::ThreadComments {
                thread_id: input.thread_id,
                cursor: input.cursor,
            },
        )
    })
    .await
}

pub(super) async fn pull_requests_diff_file_contents(
    request: Request<PullRequestDiffFileContentsInput>,
) -> Vec<Value> {
    pull_request_read(request, |mut input| {
        (
            std::mem::take(&mut input.reference),
            PullRequestRead::DiffFileContents(input),
        )
    })
    .await
}

pub(super) async fn pull_requests_checks(request: Request<PullRequestRef>) -> Vec<Value> {
    pull_request_read(request, plain(PullRequestRead::Checks)).await
}

pub(super) async fn pull_requests_change_breakdown(request: Request<PullRequestRef>) -> Vec<Value> {
    pull_request_read(request, plain(PullRequestRead::ChangeBreakdown)).await
}

pub(super) async fn pull_requests_label_candidates(request: Request<PullRequestRef>) -> Vec<Value> {
    pull_request_read(request, plain(PullRequestRead::LabelCandidates)).await
}

pub(super) async fn pull_requests_reviewer_candidates(request: Request<WriteInput>) -> Vec<Value> {
    pull_request_read(request, |mut input| {
        let query = input.query().map(str::to_string);
        (
            std::mem::take(&mut input.reference),
            PullRequestRead::ReviewerCandidates(query),
        )
    })
    .await
}

/// No stacks here: the capabilities say so, and every pull request answers
/// as in none.
pub(super) async fn pull_requests_stack(request: Request<IgnoredAny>) -> Vec<Value> {
    vec![success_exit(&request.id, Value::Null)]
}

/// A write `method` makes, split from the pull request it names.
fn write_of(method: WriteMethod) -> impl FnOnce(WriteInput) -> (PullRequestRef, PullRequestRead) {
    move |mut input| {
        (
            std::mem::take(&mut input.reference),
            PullRequestRead::Write(method, Box::new(input)),
        )
    }
}

pub(super) async fn pull_requests_comment(request: Request<WriteInput>) -> Vec<Value> {
    pull_request_read(request, write_of(WriteMethod::Comment)).await
}

pub(super) async fn pull_requests_reply_to_thread(request: Request<WriteInput>) -> Vec<Value> {
    pull_request_read(request, write_of(WriteMethod::ReplyToThread)).await
}

pub(super) async fn pull_requests_set_thread_resolution(
    request: Request<WriteInput>,
) -> Vec<Value> {
    pull_request_read(request, write_of(WriteMethod::SetThreadResolution)).await
}

pub(super) async fn pull_requests_submit_review(request: Request<WriteInput>) -> Vec<Value> {
    pull_request_read(request, write_of(WriteMethod::SubmitReview)).await
}

pub(super) async fn pull_requests_set_reaction(request: Request<WriteInput>) -> Vec<Value> {
    pull_request_read(request, write_of(WriteMethod::SetReaction)).await
}

pub(super) async fn pull_requests_update(request: Request<WriteInput>) -> Vec<Value> {
    pull_request_read(request, write_of(WriteMethod::Update)).await
}

pub(super) async fn pull_requests_update_comment(request: Request<WriteInput>) -> Vec<Value> {
    pull_request_read(request, write_of(WriteMethod::UpdateComment)).await
}

pub(super) async fn pull_requests_set_labels(request: Request<WriteInput>) -> Vec<Value> {
    pull_request_read(request, write_of(WriteMethod::SetLabels)).await
}

pub(super) async fn pull_requests_request_reviewers(request: Request<WriteInput>) -> Vec<Value> {
    pull_request_read(request, write_of(WriteMethod::RequestReviewers)).await
}

pub(super) async fn pull_requests_run_action(request: Request<WriteInput>) -> Vec<Value> {
    pull_request_read(request, write_of(WriteMethod::RunAction)).await
}

pub(super) async fn pull_requests_detail(request: Request<PullRequestRef>) -> Vec<Value> {
    pull_request_read(request, plain(PullRequestRead::Detail)).await
}

pub(super) async fn pull_requests_review_guide(request: Request<PullRequestRef>) -> Vec<Value> {
    pull_request_read(request, plain(PullRequestRead::ReviewGuide)).await
}

pub(super) async fn pull_requests_generate_review_guide(
    request: Request<PullRequestRef>,
) -> Vec<Value> {
    pull_request_read(request, plain(PullRequestRead::GenerateReviewGuide)).await
}

/// A pull request's reads and writes, split by `read` into the pull request
/// it names and what is read of or written to it.
async fn pull_request_read<P>(
    request: Request<P>,
    read: impl FnOnce(P) -> (PullRequestRef, PullRequestRead),
) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let (reference, read) = read(payload);
    let Some((project_id, repository, number)) = reference.target() else {
        return vec![bad_reference(
            &id,
            tag,
            "A pull request reference needs a projectId, repository and number.",
        )];
    };
    let target = pull_request_review::PullRequestTarget {
        project_id: &project_id,
        repository: &repository,
        number,
    };
    let ports = pull_request_review::review_ports(&state);
    let viewer = viewer_scope(&identity);
    let result = match read {
        PullRequestRead::Detail => pull_request_review::detail(&ports, &viewer, &target).await,
        PullRequestRead::ReviewGuide => {
            pull_request_review::review_guide(&ports, &viewer, &target).await
        }
        PullRequestRead::GenerateReviewGuide => {
            pull_request_review::generate_review_guide(&ports, &viewer, &target).await
        }
        PullRequestRead::Activity => {
            pull_request_conversation::activity(&ports, &viewer, &target).await
        }
        PullRequestRead::Preview => {
            pull_request_conversation::preview(&ports, &viewer, &target).await
        }
        PullRequestRead::LinkedThreads => {
            pull_request_conversation::linked_threads(&ports, &viewer, &target).await
        }
        PullRequestRead::ThreadComments { thread_id, cursor } => {
            match (
                thread_id.as_deref().map(str::trim),
                cursor.as_deref().map(str::trim),
            ) {
                (Some(thread_id), Some(cursor)) if !thread_id.is_empty() && !cursor.is_empty() => {
                    pull_request_conversation::thread_comments(
                        &ports, &viewer, &target, thread_id, cursor,
                    )
                    .await
                }
                _ => {
                    return vec![bad_reference(
                        &id,
                        tag,
                        "A thread page needs a threadId and a cursor.",
                    )];
                }
            }
        }
        PullRequestRead::DiffFileContents(input) => {
            match pull_request_conversation::changed_file(&input) {
                Some(file)
                    if file
                        .commit
                        .is_none_or(pull_request_conversation::is_commit_sha) =>
                {
                    pull_request_conversation::file_contents(&ports, &viewer, &target, &file).await
                }
                _ => {
                    return vec![bad_reference(
                        &id,
                        tag,
                        "A file read needs a changeType, oldPath and newPath, and a commit id if any.",
                    )];
                }
            }
        }
        PullRequestRead::Checks => pull_request_writes::checks(&ports, &viewer, &target).await,
        PullRequestRead::ChangeBreakdown => {
            pull_request_writes::change_breakdown(&ports, &viewer, &target).await
        }
        PullRequestRead::LabelCandidates => {
            pull_request_writes::label_candidates(&ports, &viewer, &target).await
        }
        PullRequestRead::ReviewerCandidates(query) => {
            pull_request_writes::reviewer_candidates(&ports, &viewer, &target, query.as_deref())
                .await
        }
        PullRequestRead::Write(method, input) => {
            let write = match input.into_write(method) {
                Ok(write) => write,
                Err(detail) => return vec![bad_reference(&id, tag, detail)],
            };
            match write {
                PullRequestWrite::Action(action) => {
                    pull_request_writes::run_action(&ports, &viewer, &target, action).await
                }
                write => pull_request_writes::write(&ports, &viewer, &target, &write)
                    .await
                    .map(|()| Value::Null),
            }
        }
    };
    match result {
        Ok(value) => vec![success_exit(&id, value)],
        Err(error) => vec![typed_failure_exit(
            &id,
            pull_request_review_error(tag, &error, &identity),
        )],
    }
}

/// T3's `PullRequestOperationError` for a request that names nothing to read.
fn bad_reference(id: &RequestId, tag: &str, detail: &str) -> Value {
    typed_failure_exit(
        id,
        json!({
            "_tag": "PullRequestOperationError",
            "operation": tag,
            "detail": detail,
        }),
    )
}

/// Served off a Task's own binding rather than the git host, so it answers
/// even where reading the host fails.
pub(super) async fn pull_requests_summary(request: Request<PullRequestRef>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some((project_id, repository, number)) = payload.target() else {
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

pub(super) async fn git_resolve_pull_request(
    request: Request<GitPullRequestRefInput>,
) -> Vec<Value> {
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
        Err(error) => vec![pull_request_failure_exit(&id, tag, &error, &identity)],
    }
}

pub(super) async fn git_prepare_pull_request_thread(
    request: Request<GitPreparePullRequestThreadInput>,
) -> Vec<Value> {
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
        Ok(git_actions::PreparedThread::WorktreeUnsupported) => vec![unsupported_exit(&id, tag)],
        Err(error) => vec![pull_request_failure_exit(&id, tag, &error, &identity)],
    }
}

/// Not run here or in the Sandbox.
///
/// The button press becomes a message asking the Task's agent to do the git
/// work, answered as one `action_finished` chunk plus terminal success — the
/// shape a completed action produces.
pub(super) async fn git_run_stacked_action(
    request: Request<GitRunStackedActionInput>,
) -> Vec<Value> {
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

/// The result of switching a draft thread to a branch, if this is one to
/// answer.
///
/// A no-op for a draft: there's no checkout yet, so the ref is echoed back for
/// the client to record and send at creation. `None` means refuse — a Task's
/// branch is fixed once its checkout exists.
fn branch_switch_result(input: &VcsSwitchRefInput) -> Option<Value> {
    let cwd = workspace_cwd(input.cwd.as_deref())?;
    if !nominal_path::is_project_root(&cwd) {
        return None;
    }
    let ref_name = input.ref_name.as_deref()?.trim();
    (!ref_name.is_empty()).then(|| json!({ "refName": ref_name }))
}
