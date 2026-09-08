use crate::git_actions;
use crate::oauth;
use crate::ports;
use crate::preview;
use crate::preview::PreviewError;
use crate::projection;
use crate::review;
use crate::rpc::metrics as rpc_metrics;
use crate::sandbox::SandboxRpcError;
use crate::scripts::ScriptsError;
use crate::servers::ServersError;
use crate::snapshots::workspace_cwd;
use crate::subtasks::SubtasksError;
use crate::terminal;
use crate::workspace::WorkspaceAccessError;
use moatless_ports::sandbox::SandboxError;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::fmt;
use tracing::debug;
use tracing::error;
use tracing::warn;

/// A request id, in the JSON form it arrived as and the text this protocol
/// correlates on.
///
/// T3 encodes the id as `string | number` and sends whichever it already had —
/// an unquoted integer for the counter its client generates. The form matters
/// on the way back: the client resolves a response by looking `requestId` up in
/// a map keyed by the value it sent, and its JSON codec coerces nothing. An id
/// that arrives as `0` and returns as `"0"` therefore matches no caller, the
/// request never completes, and the connection attempt times out with the
/// socket open and every frame apparently fine. So the id goes back exactly as
/// it came, while correlation here stays on the decimal text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RequestId {
    pub(crate) text: String,
    pub(crate) wire: Value,
}

impl RequestId {
    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    pub(crate) fn wire(&self) -> Value {
        self.wire.clone()
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct RequestIdVisitor;

        impl serde::de::Visitor<'_> for RequestIdVisitor {
            type Value = RequestId;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a request id as a string or an integer")
            }

            fn visit_str<E: serde::de::Error>(self, id: &str) -> Result<RequestId, E> {
                Ok(RequestId {
                    text: id.to_string(),
                    wire: Value::String(id.to_string()),
                })
            }

            fn visit_u64<E: serde::de::Error>(self, id: u64) -> Result<RequestId, E> {
                Ok(RequestId {
                    text: id.to_string(),
                    wire: Value::from(id),
                })
            }

            fn visit_i64<E: serde::de::Error>(self, id: i64) -> Result<RequestId, E> {
                Ok(RequestId {
                    text: id.to_string(),
                    wire: Value::from(id),
                })
            }
        }

        // A fractional id reaches no visitor here and is refused, which is what
        // `valid_request_id` would decide about it anyway.
        deserializer.deserialize_any(RequestIdVisitor)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "_tag")]
pub(crate) enum ClientFrame {
    Request {
        id: RequestId,
        tag: String,
        #[serde(default)]
        payload: Value,
        #[serde(default)]
        headers: Vec<(String, String)>,
        #[serde(rename = "traceId")]
        #[serde(default)]
        trace_id: Option<String>,
        #[serde(rename = "spanId")]
        #[serde(default)]
        span_id: Option<String>,
        #[serde(default)]
        sampled: Option<bool>,
    },
    Ack {
        #[serde(rename = "requestId")]
        request_id: RequestId,
    },
    Interrupt {
        #[serde(rename = "requestId")]
        request_id: RequestId,
    },
    Ping,
    Eof,
}

pub(crate) fn success_exit(request_id: &RequestId, value: Value) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Success",
            "value": value
        }
    })
}

pub(crate) fn failure_exit(request_id: &RequestId, defect: impl Into<String>) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Failure",
            "cause": [{
                "_tag": "Die",
                "defect": defect.into()
            }]
        }
    })
}

/// Fail a request with an error the client's schema knows how to decode.
///
/// [`failure_exit`] dies with a string, which every caller renders as an
/// unexpected server defect. A `Fail` cause carrying a tagged error is instead
/// matched against the method's declared error type, so the client handles it as
/// one of that method's ordinary outcomes. Use this where the failure is one the
/// contract anticipated and the string form where it is not.
pub(crate) fn typed_failure_exit(request_id: &RequestId, error: Value) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Failure",
            "cause": [{
                "_tag": "Fail",
                "error": error
            }]
        }
    })
}

/// What a viewer is told about a Task the read gate did not return.
///
/// It says the same thing whether the Task is missing or merely unreadable,
/// because `find_by_id` answers `None` to both and the difference is what a
/// caller would use to probe for Tasks it cannot see.
pub(crate) const THREAD_UNAVAILABLE: &str = "This thread is not available.";

/// The contract's `EnvironmentAuthorizationError`, as the error a `Fail` cause
/// carries.
///
/// `requiredScope` is the scope the method needs, which is not always why the
/// request was refused: a viewer holding the scope may still be refused one
/// Task. The `message` is what says which of the two happened.
pub(crate) fn authorization_error(required_scope: &str, message: impl Into<String>) -> Value {
    json!({
        "_tag": "EnvironmentAuthorizationError",
        "message": message.into(),
        "requiredScope": required_scope,
    })
}

/// A workspace-file request the client is told about in its own terms.
///
/// This is one of the failures [`typed_failure_exit`] exists for, and the
/// difference from a `Die` is what the person sees. Each of these methods
/// declares a tagged error in the contract, and the client renders that
/// error's `message` in the file panel; a `Die` reaches the same panel as
/// nothing but *"Workspace query failed."*, because the defect never decodes
/// into an `Error` for it to read a message off
/// (`projectFilesQueryState.ts:errorMessage`). Refusing to open a file is a
/// normal outcome here — no sandbox, a binary file, a stopped pod — and each of
/// those is worth saying out loud.
///
/// The tag names which error type the method declares, since the client decodes
/// the cause against that method's own union and a mismatch would land back as
/// an undecodable defect. Causes stay in the log: the `message` is written for
/// the person, and the sqlx or gRPC failure behind it is not.
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
/// `GitCommandError` is the one shape both methods' error unions carry, and the
/// client renders its `message` — `Git command failed in {operation} ({cwd}):
/// {detail}` — in the diff panel. A `Die` would reach the same panel as nothing
/// but an unexpected server error, and refusing to show a diff is a normal
/// outcome here: no sandbox, a stopped pod, a conversation this viewer cannot
/// be told about.
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
/// A refusal the viewer caused — an unaddressable `cwd`, a Task they do not
/// control, a malformed request — or the message failing to send after it was
/// accepted. A rejected request is a `Die` (the client sent something it should
/// not have); an access refusal or a failed send carries the same
/// `GitCommandError` shape the read path uses, so it renders in the toolbar the
/// same way a status read's failure does.
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

/// Why a pull-request reference could not be resolved, as the failure both
/// dialog methods declare.
///
/// `GitManagerError` is one shape of their `GitManagerServiceError` union, and
/// the client renders its `message` — `Git manager failed in {operation}:
/// {detail}` — in the attach-PR dialog. A reference that names no reachable pull
/// request, or a repository the viewer cannot be shown, is an ordinary outcome
/// there; a database failure is logged and reported the same way, since a `Die`
/// would reach the dialog as an undecodable defect.
/// A `pullRequests.summary` refusal, as the contract's `PullRequestRpcError`
/// union carries one. `PullRequestUnavailableError` is the wrong tag: its
/// reasons all name a git host this method never reaches.
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

/// The `PullRequestRef` a summary request carries.
pub(crate) fn pull_request_ref(payload: &Value) -> Option<(String, String, u64)> {
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let number = payload.get("number").and_then(Value::as_u64)?;
    if number == 0 {
        return None;
    }
    Some((text("projectId")?, text("repository")?, number))
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
/// A workspace with no repository the viewer can be shown answers with an empty
/// list, the same as a conversation with no Sandbox. The other outcomes are the
/// branch picker's own: an unavailable git host — no token, a host that refused
/// — is surfaced verbatim so the "connect your account" prompt lands, and a
/// database failure is logged and reported as a generic git error, since
/// `GitCommandError` is the shape this method declares.
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

/// The result of switching a draft thread to a branch, if this is one to
/// answer.
///
/// The switch is a no-op for a draft: there is no checkout yet, so the ref is
/// echoed back for the client to record on the draft and send at creation.
/// `None` — a switch on a thread that already has a Sandbox, or one with no
/// addressable `cwd` — is left for the caller to refuse, because a Task's
/// branch is fixed once its checkout exists.
pub(crate) fn branch_switch_result(payload: &Value) -> Option<Value> {
    let cwd = workspace_cwd(payload)?;
    if !projection::is_project_root(&cwd) {
        return None;
    }
    let ref_name = payload.get("refName").and_then(Value::as_str)?.trim();
    (!ref_name.is_empty()).then(|| json!({ "refName": ref_name }))
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
/// An unknown tab is a `PreviewSessionLookupError` the panel renders; a store
/// that cannot be reached is a defect, because it is not one of the method's
/// declared outcomes and pretending otherwise would present an outage as a
/// missing tab.
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

/// The client is told the servers could not be read; the cause is logged once
/// here rather than shipped to the browser.
pub(crate) fn servers_failure_exit(
    request_id: &RequestId,
    error: &ServersError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    error!(%error, user_id = %identity.user_id, "Failed to read a Task's servers");
    failure_exit(request_id, "Failed to read the conversation's servers")
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
/// A terminal failure that means the Sandbox is stopped is retold as
/// [`sandbox_not_running`], which is what this method's contract declares; the
/// rest of the terminal errors are not in that union and reach the client as a
/// string. A reserved-port refusal is a configuration mistake and its message is
/// shown verbatim; a database error is logged here and generalised.
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

/// The contract's `SandboxNotRunningError`, as the error a `Fail` cause carries.
///
/// Typed, not a string: this is the one `scripts.run` refusal a client can *act*
/// on rather than report — start the Sandbox, wait for it, and run the Script
/// the person already asked for. A `Die` with a sentence in it would leave that
/// behind string matching.
fn sandbox_not_running(thread_id: &str) -> Value {
    json!({
        "_tag": "SandboxNotRunningError",
        "threadId": thread_id,
    })
}

/// The client is told the sandbox status or command could not be answered; the
/// cause is logged once here rather than shipped to the browser.
///
/// [`SandboxRpcError::NotFound`] is the one that is not a defect. It is what the
/// read gate answers for a Task this viewer may not be told about, which is a
/// decision this surface made and which every sandbox method declares as
/// `EnvironmentAuthorizationError` — so it goes back as a `Fail` the client
/// renders, not a `Die` it can only report as a broken server.
pub(crate) fn sandbox_failure_exit(
    request_id: &RequestId,
    tag: &str,
    error: &SandboxRpcError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    if matches!(error, SandboxRpcError::NotFound) {
        warn!(%tag, user_id = %identity.user_id, "T3 UI sandbox request refused; thread not readable");
        return typed_failure_exit(
            request_id,
            authorization_error(oauth::required_scope(tag), THREAD_UNAVAILABLE),
        );
    }
    error!(%error, user_id = %identity.user_id, "Failed to answer a Task's sandbox lifecycle request");
    failure_exit(request_id, "Failed to read the conversation's sandbox")
}

/// How `sandbox.subscribeStatus` ends once its Task stops being readable.
///
/// The same refusal [`sandbox_failure_exit`] answers the opening request with,
/// as the `Exit` that closes a stream already delivering chunks. The caller
/// logs it: this is reached from the poller, which knows the Task id.
pub(crate) fn sandbox_stream_refusal(request_id: &RequestId) -> Value {
    typed_failure_exit(
        request_id,
        authorization_error(
            oauth::required_scope("sandbox.subscribeStatus"),
            THREAD_UNAVAILABLE,
        ),
    )
}

/// A conversation with no Sandbox, as `servers.list` answers it.
///
/// `servers.list` declares no domain error, so there is no typed failure to give
/// here and a `Die` would reach the panel as an unexpected server error. The
/// server-list contract names this case itself: an empty list is what a project
/// with no sandbox has to list. Sandbox lifecycle status is answered separately
/// by `sandbox.status`.
pub(crate) fn servers_not_found() -> Value {
    json!({ "servers": [] })
}

/// The method is not one this server has.
///
/// Every RPC declares `UnsupportedMethodError`, so this decodes wherever it
/// lands and the client can say which method is absent, distinguishing a
/// server that does not implement a method from one that has failed.
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

pub(crate) fn interrupted_exit(request_id: &RequestId) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Failure",
            "cause": [{
                "_tag": "Interrupt"
            }]
        }
    })
}

/// Refuse a frame the socket could not act on at all.
///
/// A `Defect` answers the connection rather than a request, so it is what the
/// checks that run before any dispatch emit. `reason` is a fixed word, not the
/// message: the message carries a serde error or a request id and would make the
/// counter's label set unbounded.
pub(crate) fn connection_defect(reason: &'static str, defect: impl Into<String>) -> Value {
    metrics::counter!(rpc_metrics::CONNECTION_DEFECTS, "reason" => reason).increment(1);
    json!({
        "_tag": "Defect",
        "defect": defect.into()
    })
}
pub(crate) fn valid_request_id(id: &str) -> bool {
    let digits = id.strip_prefix('-').unwrap_or(id);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}
