//! How each domain failure reaches the client: the T3 contract error it
//! becomes, and the one log line its cause gets.

use crate::git_actions;
use crate::oauth::Scope;
use crate::ports;
use crate::preview;
use crate::preview::PreviewError;
use crate::review;
use crate::rpc::envelope::RequestId;
use crate::rpc::envelope::failure_exit;
use crate::rpc::envelope::success_exit;
use crate::rpc::envelope::typed_failure_exit;
use crate::sandbox::SandboxRpcError;
use crate::scripts::ScriptsError;
use crate::subtasks::SubtasksError;
use crate::terminal;
use crate::workspace::WorkspaceAccessError;
use moatless_ports::sandbox::SandboxError;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde_json::Value;
use serde_json::json;
use tracing::debug;
use tracing::error;
use tracing::warn;

/// What a viewer is told about a Task the read gate did not return.
///
/// Says the same thing whether missing or merely unreadable, so a caller can't
/// probe for Tasks it cannot see.
pub(crate) const THREAD_UNAVAILABLE: &str = "This thread is not available.";

/// The contract's `EnvironmentAuthorizationError`, as the error a `Fail` cause
/// carries.
///
/// `requiredScope` is not always why the request was refused — a viewer with
/// the scope may still be refused one Task; `message` says which happened.
pub(crate) fn authorization_error(required_scope: Scope, message: impl Into<String>) -> Value {
    json!({
        "_tag": "EnvironmentAuthorizationError",
        "message": message.into(),
        "requiredScope": required_scope.as_str(),
    })
}

/// A workspace-file request the client is told about in its own terms.
///
/// The client renders `message` in the file panel (a `Die` would show only a
/// generic error); `tag` selects the error type and must match the method's own
/// declared union or the cause fails to decode.
pub(crate) fn file_failure_exit(
    request_id: &RequestId,
    tag: &str,
    error: &WorkspaceAccessError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    let error_tag = match tag {
        "projects.readFile" => "ProjectReadFileError",
        "projects.searchEntries" => "ProjectSearchEntriesError",
        "projects.writeFile" => "ProjectWriteFileError",
        _ => "ProjectListEntriesError",
    };
    match error {
        WorkspaceAccessError::Database(cause) => {
            error!(%cause, %tag, user_id = %identity.user_id, "T3 UI workspace file request failed");
        }
        WorkspaceAccessError::Sandbox(cause) => {
            warn!(%cause, %tag, user_id = %identity.user_id, "T3 UI workspace file request failed");
        }
        // A sandbox that is not up is the ordinary state of most conversations,
        // so this one is not a warning about the deployment.
        WorkspaceAccessError::NotRunning(cause) => {
            debug!(%cause, %tag, user_id = %identity.user_id, "T3 UI workspace file request refused");
        }
        WorkspaceAccessError::NoWorktree
        | WorkspaceAccessError::Unaddressable
        | WorkspaceAccessError::NoControl => {
            debug!(?error, %tag, user_id = %identity.user_id, "T3 UI workspace file request refused");
        }
    }
    typed_failure_exit(
        request_id,
        json!({ "_tag": error_tag, "message": error.message() }),
    )
}

/// Why a review request could not be answered, as the failure its method
/// declares.
///
/// `GitCommandError`'s `message` is what the client renders in the diff panel;
/// a `Die` would show only a generic server error.
pub(crate) fn review_failure_exit(
    request_id: &RequestId,
    tag: &str,
    cwd: &str,
    error: &WorkspaceAccessError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    match error {
        WorkspaceAccessError::Database(cause) => {
            error!(%cause, %tag, user_id = %identity.user_id, "T3 UI review request failed");
        }
        WorkspaceAccessError::Sandbox(cause) => {
            warn!(%cause, %tag, user_id = %identity.user_id, "T3 UI review request failed");
        }
        // A sandbox that is not up is the ordinary state of most conversations,
        // so this one is not a warning about the deployment.
        WorkspaceAccessError::NotRunning(cause) => {
            debug!(%cause, %tag, user_id = %identity.user_id, "T3 UI review request refused");
        }
        WorkspaceAccessError::NoWorktree
        | WorkspaceAccessError::Unaddressable
        | WorkspaceAccessError::NoControl => {
            debug!(?error, %tag, user_id = %identity.user_id, "T3 UI review request refused");
        }
    }
    // `command` names what was actually attempted. Nothing here shells out to
    // git: the Sandbox answers these over gRPC, and reporting `git` as the
    // command sends anyone reading a support log looking for a shell invocation
    // that does not exist.
    let command = match tag {
        "vcs.listRefs" => "sandbox.ListGitRefs",
        "vcs.refreshStatus" => "sandbox.GetGitStatus",
        _ => "sandbox.GetGitDiffPreview",
    };
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "GitCommandError",
            "operation": tag,
            "command": command,
            "cwd": cwd,
            "detail": error.message(),
        }),
    )
}

/// Refuse a `git.runStackedAction` that could not be handed to the agent.
///
/// A rejected request is a `Die` (the client sent something it shouldn't have);
/// an access refusal or a failed send carries the same `GitCommandError` shape
/// the read path uses.
pub(crate) fn git_action_failure_exit(
    request_id: &RequestId,
    error: &git_actions::ActionError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    let detail = match error {
        git_actions::ActionError::Rejected(message) => {
            debug!(%message, user_id = %identity.user_id, "T3 git action rejected");
            return failure_exit(request_id, message.clone());
        }
        git_actions::ActionError::Access(access) => {
            match access {
                WorkspaceAccessError::Database(cause) => {
                    error!(%cause, user_id = %identity.user_id, "T3 git action failed");
                }
                WorkspaceAccessError::Sandbox(cause) => {
                    warn!(%cause, user_id = %identity.user_id, "T3 git action failed");
                }
                WorkspaceAccessError::NotRunning(cause) => {
                    debug!(%cause, user_id = %identity.user_id, "T3 git action refused");
                }
                WorkspaceAccessError::NoWorktree
                | WorkspaceAccessError::Unaddressable
                | WorkspaceAccessError::NoControl => {
                    debug!(?access, user_id = %identity.user_id, "T3 git action refused");
                }
            }
            access.message()
        }
        git_actions::ActionError::Send(refusal) => {
            warn!(
                reason = %refusal.message(),
                user_id = %identity.user_id,
                "T3 git action could not ask the agent"
            );
            refusal.message()
        }
    };
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "GitCommandError",
            "operation": "git.runStackedAction",
            "command": "orchestration.sendMessage",
            "cwd": "",
            "detail": detail,
        }),
    )
}

/// Why a `pullRequests.summary` request could not be answered.
///
/// The contract's `PullRequestRpcError` union carries this as
/// `PullRequestOperationError`; `PullRequestUnavailableError` is the wrong tag,
/// since its reasons all name a git host this method never reaches.
pub(crate) fn pull_request_summary_failure_exit(request_id: &RequestId, detail: &str) -> Value {
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "PullRequestOperationError",
            "operation": "pullRequests.summary",
            "detail": detail,
        }),
    )
}

pub(crate) fn pull_request_failure_exit(
    request_id: &RequestId,
    tag: &str,
    error: &ports::PullRequestError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    let detail = match error {
        ports::PullRequestError::NotFound => {
            "No pull request was found for that reference.".to_string()
        }
        ports::PullRequestError::Unavailable(message) => message.clone(),
        ports::PullRequestError::Failed(cause) => {
            error!(%cause, %tag, user_id = %identity.user_id, "T3 pull-request resolution failed");
            "The pull request could not be resolved.".to_string()
        }
    };
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "GitManagerError",
            "operation": tag,
            // Nothing here shells out to git: the pull request is read from the
            // git host's API, so this names the read rather than a checkout.
            "cwd": "",
            "detail": detail,
        }),
    )
}

/// Answer a project-root `vcs.listRefs` whose branch listing did not produce a
/// page.
///
/// An empty repository answers with an empty list; an unavailable git host is
/// surfaced verbatim so the "connect your account" prompt lands; a database
/// failure is logged and generalized.
pub(crate) fn branch_list_exit(
    request_id: &RequestId,
    cwd: &str,
    error: ports::BranchListError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    let detail = match error {
        ports::BranchListError::None => return success_exit(request_id, review::no_refs()),
        ports::BranchListError::Unavailable(message) => message,
        ports::BranchListError::Failed(cause) => {
            error!(%cause, user_id = %identity.user_id, "T3 UI branch listing failed");
            "The branches could not be listed.".to_string()
        }
    };
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "GitCommandError",
            "operation": "vcs.listRefs",
            // Nothing here shells out to git: the branches come from the git
            // host's API, not a checkout, so this names the read rather than a
            // shell invocation that does not exist.
            "command": "repository.ListBranches",
            "cwd": cwd,
            "detail": detail,
        }),
    )
}

/// The client is told the listing could not be built; the cause is logged once
/// here rather than shipped to the browser.
pub(crate) fn shell_failure_exit(
    request_id: &RequestId,
    error: &sqlx::Error,
    identity: &UiRpcAuthIdentity,
) -> Value {
    error!(%error, user_id = %identity.user_id, "Failed to assemble T3 UI shell snapshot");
    failure_exit(request_id, "Failed to assemble the shell listing")
}

/// Answer a `preview.*` call, in the client's own terms where the contract
/// gives it terms.
///
/// An unknown tab is a `PreviewSessionLookupError`; a store that can't be
/// reached is a defect, since that isn't one of the method's declared outcomes.
pub(crate) fn preview_answer(
    request_id: &RequestId,
    result: Result<Value, PreviewError>,
) -> Vec<Value> {
    match result {
        Ok(value) => vec![success_exit(request_id, value)],
        Err(error) => {
            if let Some(typed) = preview::typed_error(&error) {
                vec![typed_failure_exit(request_id, typed)]
            } else {
                warn!(%error, "A T3 UI preview call failed");
                vec![failure_exit(request_id, error.to_string())]
            }
        }
    }
}

pub(crate) fn terminal_failure_exit(
    request_id: &RequestId,
    payload: &Value,
    error: &terminal::TerminalError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    log_terminal_failure(error, identity);
    match terminal::typed_error(error, payload) {
        Some(error) => typed_failure_exit(request_id, error),
        None => failure_exit(request_id, terminal::message(error)),
    }
}

fn log_terminal_failure(error: &terminal::TerminalError, identity: &UiRpcAuthIdentity) {
    match error {
        terminal::TerminalError::Workspace(WorkspaceAccessError::Database(cause)) => {
            error!(%cause, user_id = %identity.user_id, "T3 UI terminal request failed");
        }
        terminal::TerminalError::Workspace(WorkspaceAccessError::Sandbox(cause))
        | terminal::TerminalError::Workspace(WorkspaceAccessError::NotRunning(cause))
        | terminal::TerminalError::Sandbox(cause) => {
            warn!(%cause, user_id = %identity.user_id, "T3 UI terminal request failed");
        }
        terminal::TerminalError::Workspace(
            WorkspaceAccessError::NoWorktree
            | WorkspaceAccessError::Unaddressable
            | WorkspaceAccessError::NoControl,
        )
        | terminal::TerminalError::Malformed(_) => {
            debug!(?error, user_id = %identity.user_id, "T3 UI terminal request refused");
        }
    }
}

/// The client is told the subtasks could not be read; the cause is logged once
/// here rather than shipped to the browser.
pub(crate) fn subtasks_failure_exit(
    request_id: &RequestId,
    error: &SubtasksError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    error!(%error, user_id = %identity.user_id, "Failed to read a Task's subtasks");
    failure_exit(request_id, "Failed to read the conversation's subtasks")
}

/// How a failed `scripts.run` reaches the client.
///
/// A terminal failure meaning the Sandbox is stopped becomes
/// [`sandbox_not_running`], the contract's declared outcome; other terminal
/// errors reach the client as a plain string; a reserved-port refusal's message
/// is shown verbatim.
pub(crate) fn scripts_failure_exit(
    request_id: &RequestId,
    error: &ScriptsError,
    payload: &Value,
    identity: &UiRpcAuthIdentity,
) -> Value {
    let thread_id = payload
        .get("threadId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match error {
        ScriptsError::Terminal(terminal_error) => {
            log_terminal_failure(terminal_error, identity);
            // `scripts.run` does not declare the `Terminal*` tags, so one of
            // them reaches the browser as a `Fail` the client cannot decode.
            if terminal::means_not_running(terminal_error) {
                typed_failure_exit(request_id, sandbox_not_running(thread_id))
            } else {
                failure_exit(request_id, terminal::message(terminal_error))
            }
        }
        ScriptsError::Port(SandboxError::NotReady(_)) => {
            debug!(?error, user_id = %identity.user_id, "scripts.run refused: sandbox not running");
            typed_failure_exit(request_id, sandbox_not_running(thread_id))
        }
        ScriptsError::Port(SandboxError::BadRequest(detail)) => {
            debug!(%detail, user_id = %identity.user_id, "scripts.run refused a reserved port");
            failure_exit(request_id, detail.clone())
        }
        ScriptsError::Port(cause) => {
            warn!(%cause, user_id = %identity.user_id, "scripts.run could not publish the port");
            failure_exit(request_id, "The script's port could not be published.")
        }
        ScriptsError::ScriptNotFound(name) => {
            debug!(%name, user_id = %identity.user_id, "scripts.run: no such script");
            failure_exit(request_id, format!("No script named \"{name}\"."))
        }
        ScriptsError::Sandbox(SandboxError::NotReady(_) | SandboxError::NotFound(_)) => {
            debug!(?error, user_id = %identity.user_id, "scripts.run refused: sandbox not running");
            typed_failure_exit(request_id, sandbox_not_running(thread_id))
        }
        ScriptsError::Sandbox(SandboxError::BadRequest(detail)) => {
            debug!(%detail, user_id = %identity.user_id, "scripts.run refused the task's own scripts");
            failure_exit(request_id, detail.clone())
        }
        ScriptsError::Sandbox(cause) => {
            warn!(%cause, user_id = %identity.user_id, "A task's own scripts could not be reached");
            failure_exit(request_id, "The task's own scripts could not be reached.")
        }
        ScriptsError::Invalid(detail) => failure_exit(request_id, detail.clone()),
        ScriptsError::NotFound => {
            debug!(user_id = %identity.user_id, "scripts.run: conversation not found");
            failure_exit(request_id, "Conversation not found.")
        }
        ScriptsError::Malformed(field) => {
            failure_exit(request_id, format!("scripts.run requires {field}"))
        }
        ScriptsError::Database(cause) => {
            error!(%cause, user_id = %identity.user_id, "scripts.run failed to read the conversation");
            failure_exit(request_id, "Failed to read the conversation.")
        }
    }
}

/// How a failed `scripts.listForThread` reaches the client.
///
/// Only a database error gets here: `NotFound` is answered as an empty listing
/// by the caller, and an unreachable sandbox costs the Task's own Scripts
/// rather than the listing ([`Resolved::own_unread`](crate::scripts::Resolved)).
pub(crate) fn scripts_list_failure_exit(
    request_id: &RequestId,
    error: &ScriptsError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    error!(%error, user_id = %identity.user_id, "scripts.listForThread failed");
    failure_exit(request_id, "Failed to list the conversation's scripts.")
}

/// The contract's `SandboxNotRunningError`, as the error a `Fail` cause
/// carries.
///
/// Typed, not a string: this is the one `scripts.run` refusal a client can act
/// on — start the Sandbox, wait, then run the Script — rather than just report.
fn sandbox_not_running(thread_id: &str) -> Value {
    json!({
        "_tag": "SandboxNotRunningError",
        "threadId": thread_id,
    })
}

/// The client is told the sandbox status or command could not be answered; the
/// cause is logged once here rather than shipped to the browser.
///
/// [`SandboxRpcError::NotFound`] is not a defect: it's what the read gate
/// answers for a Task this viewer may not be told about, so it goes back as
/// `EnvironmentAuthorizationError`.
pub(crate) fn sandbox_failure_exit(
    request_id: &RequestId,
    scope: Scope,
    tag: &str,
    error: &SandboxRpcError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    match error {
        SandboxRpcError::NotFound => {
            warn!(%tag, user_id = %identity.user_id, "T3 UI sandbox request refused; thread not readable");
            typed_failure_exit(request_id, authorization_error(scope, THREAD_UNAVAILABLE))
        }
        // Moatless declined the change and said why in terms the person asking
        // can act on, so its own words go back rather than the generic failure.
        SandboxRpcError::Refused(message) => {
            warn!(%tag, %message, user_id = %identity.user_id, "T3 UI sandbox request refused by Moatless");
            failure_exit(request_id, message.as_str())
        }
        SandboxRpcError::Sandbox(_) | SandboxRpcError::Database(_) => {
            error!(%error, user_id = %identity.user_id, "Failed to answer a Task's sandbox lifecycle request");
            failure_exit(request_id, "Failed to read the conversation's sandbox")
        }
    }
}

/// How a `sandbox.subscribe*` stream ends once its Task stops being readable.
///
/// The same refusal [`sandbox_failure_exit`] gives the opening request, as the
/// `Exit` that closes a stream already delivering chunks.
pub(crate) fn sandbox_stream_refusal(request_id: &RequestId, scope: Scope) -> Value {
    typed_failure_exit(request_id, authorization_error(scope, THREAD_UNAVAILABLE))
}

/// The method is not one this server has.
///
/// Every RPC declares `UnsupportedMethodError`, so this decodes wherever it
/// lands and the client can say which method is absent.
pub(crate) fn unsupported_exit(request_id: &RequestId, tag: &str) -> Value {
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "UnsupportedMethodError",
            "method": tag,
            "message": format!("{tag} is not available on this server."),
        }),
    )
}
