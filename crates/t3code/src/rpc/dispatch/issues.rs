//! `issues.*` and `issueTrackers.*`: the Linear team each Workspace declares.

use crate::authentication::viewer_scope;
use crate::issues;
use crate::issues::{IssueFailure, IssueListInput, IssueRequest, ThreadIssueKeyInput};
use crate::oauth::Scope;
use crate::rpc::envelope::success_exit;
use crate::rpc::envelope::typed_failure_exit;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use serde::de::IgnoredAny;
use serde_json::Value;

pub(super) fn issue_methods() -> MethodTable {
    MethodTable::new()
        .detached("issues.list", Scope::OrchestrationRead, issues_list)
        .detached("issues.detail", Scope::OrchestrationRead, issues_detail)
        .detached("issues.comments", Scope::OrchestrationRead, issues_comments)
        .detached(
            "issues.templates",
            Scope::OrchestrationRead,
            issues_templates,
        )
        .detached(
            "issues.candidates",
            Scope::OrchestrationRead,
            issues_candidates,
        )
        .detached(
            "issues.linkedThreads",
            Scope::OrchestrationRead,
            issues_linked_threads,
        )
        .detached("issues.create", Scope::OrchestrationOperate, issues_create)
        .detached(
            "issues.comment",
            Scope::OrchestrationOperate,
            issues_comment,
        )
        .detached("issues.update", Scope::OrchestrationOperate, issues_update)
        .detached(
            "issues.setState",
            Scope::OrchestrationOperate,
            issues_set_state,
        )
        .detached(
            "issueTrackers.list",
            Scope::OrchestrationRead,
            issue_trackers_list,
        )
}

fn answer(
    request_id: &crate::rpc::envelope::RequestId,
    tag: &str,
    result: Result<Value, IssueFailure>,
) -> Vec<Value> {
    match result {
        Ok(value) => vec![success_exit(request_id, value)],
        Err(failure) => vec![typed_failure_exit(
            request_id,
            issues::failure_json(tag, &failure),
        )],
    }
}

async fn issues_list(request: Request<IssueListInput>) -> Vec<Value> {
    let ports = issues::issue_ports(&request.state);
    let viewer = viewer_scope(&request.identity);
    let result = issues::list(&ports, &viewer, &request.payload).await;
    answer(&request.id, request.tag, result)
}

async fn issues_detail(request: Request<IssueRequest>) -> Vec<Value> {
    let ports = issues::issue_ports(&request.state);
    let viewer = viewer_scope(&request.identity);
    let result = issues::detail(&ports, &viewer, &request.payload).await;
    answer(&request.id, request.tag, result)
}

async fn issues_comments(request: Request<IssueRequest>) -> Vec<Value> {
    let ports = issues::issue_ports(&request.state);
    let viewer = viewer_scope(&request.identity);
    let result = issues::comments(&ports, &viewer, &request.payload).await;
    answer(&request.id, request.tag, result)
}

async fn issues_templates(request: Request<IssueRequest>) -> Vec<Value> {
    let ports = issues::issue_ports(&request.state);
    let viewer = viewer_scope(&request.identity);
    let result = issues::templates(&ports, &viewer, &request.payload).await;
    answer(&request.id, request.tag, result)
}

async fn issues_candidates(request: Request<IssueRequest>) -> Vec<Value> {
    let ports = issues::issue_ports(&request.state);
    let viewer = viewer_scope(&request.identity);
    let result = issues::candidates(&ports, &viewer, &request.payload).await;
    answer(&request.id, request.tag, result)
}

async fn issues_linked_threads(request: Request<ThreadIssueKeyInput>) -> Vec<Value> {
    let ports = issues::issue_ports(&request.state);
    let viewer = viewer_scope(&request.identity);
    let result = issues::linked_threads(&ports, &viewer, &request.payload).await;
    answer(&request.id, request.tag, result)
}

async fn issues_create(request: Request<IssueRequest>) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let ports = issues::issue_ports(&state);
    let viewer = viewer_scope(&identity);
    let result = issues::create(&ports, &viewer, &identity.login, payload).await;
    answer(&id, tag, result)
}

async fn issues_comment(request: Request<IssueRequest>) -> Vec<Value> {
    let ports = issues::issue_ports(&request.state);
    let viewer = viewer_scope(&request.identity);
    let result = issues::comment(&ports, &viewer, &request.identity.login, &request.payload).await;
    answer(&request.id, request.tag, result)
}

async fn issues_update(request: Request<IssueRequest>) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let ports = issues::issue_ports(&state);
    let viewer = viewer_scope(&identity);
    let result = issues::update(&ports, &viewer, payload).await;
    answer(&id, tag, result)
}

async fn issues_set_state(request: Request<IssueRequest>) -> Vec<Value> {
    let ports = issues::issue_ports(&request.state);
    let viewer = viewer_scope(&request.identity);
    let result = issues::set_state(&ports, &viewer, &request.payload).await;
    answer(&request.id, request.tag, result)
}

async fn issue_trackers_list(request: Request<IgnoredAny>) -> Vec<Value> {
    let ports = issues::issue_ports(&request.state);
    let viewer = viewer_scope(&request.identity);
    let result = issues::trackers(&ports, &viewer).await;
    answer(&request.id, request.tag, result)
}
