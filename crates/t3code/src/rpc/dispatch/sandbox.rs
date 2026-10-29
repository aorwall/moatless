//! Methods that act on a thread's Sandbox and read what runs inside it.

use crate::authentication::viewer_scope;
use crate::oauth::Scope;
use crate::rpc::envelope::success_exit;
use crate::rpc::errors::sandbox_failure_exit;
use crate::rpc::errors::scripts_failure_exit;
use crate::rpc::errors::scripts_list_failure_exit;
use crate::rpc::errors::subtasks_failure_exit;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use crate::sandbox;
use crate::scripts;
use crate::scripts::ScriptsError;
use crate::scripts::ScriptsRunInput;
use crate::subtasks;
use crate::subtasks::SubtasksError;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Value;
use serde_json::json;

pub(super) fn sandbox_methods() -> MethodTable {
    MethodTable::new()
        .session(
            "sandbox.cleanup",
            Scope::OrchestrationOperate,
            sandbox_cleanup,
        )
        .detached(
            "sandbox.listDiagnostics",
            Scope::OrchestrationRead,
            sandbox_list_diagnostics,
        )
        .session(
            "sandbox.redeploy",
            Scope::OrchestrationOperate,
            sandbox_redeploy,
        )
        .session(
            "sandbox.restart",
            Scope::OrchestrationOperate,
            sandbox_restart,
        )
        .session(
            "sandbox.setIdleTimeout",
            Scope::OrchestrationOperate,
            sandbox_set_idle_timeout,
        )
        .session("sandbox.start", Scope::OrchestrationRead, sandbox_start)
        .detached("sandbox.status", Scope::OrchestrationRead, sandbox_status)
        .session("sandbox.stop", Scope::OrchestrationRead, sandbox_stop)
        .detached(
            "scripts.listForThread",
            Scope::OrchestrationRead,
            scripts_list_for_thread,
        )
        .session("scripts.run", Scope::OrchestrationRead, scripts_run)
        .detached("subtasks.list", Scope::OrchestrationRead, subtasks_list)
}

/// The contract's `SandboxStatusInput`, and every other `sandbox.*` input that
/// names only a thread (`SandboxDetailInput`, `SandboxStartInput`, …).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SandboxStatusInput {
    pub(super) thread_id: String,
}

/// The contract's `ScriptsListForThreadInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScriptsListForThreadInput {
    thread_id: String,
}

/// The contract's `SubtasksListInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SubtasksListInput {
    thread_id: String,
}

/// The contract's `SandboxSetIdleTimeoutInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SandboxSetIdleTimeoutInput {
    thread_id: String,
    /// `i32` and not `u32`: a client that sends a negative gets the refusal
    /// `sandbox::set_idle_timeout` states, not a decode failure.
    minutes: i32,
}

pub(super) async fn scripts_run(request: Request<ScriptsRunInput>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match scripts::press(
        &state.scripts,
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &state.terminal,
        &viewer,
        &payload,
    )
    .await
    {
        Ok(result) => vec![success_exit(&id, result)],
        Err(error) => vec![scripts_failure_exit(
            &id,
            &error,
            &payload.thread_id,
            &identity,
        )],
    }
}

pub(super) async fn scripts_list_for_thread(
    request: Request<ScriptsListForThreadInput>,
) -> Vec<Value> {
    let Request {
        id,
        payload: ScriptsListForThreadInput { thread_id: task_id },
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match scripts::list(&state.scripts, state.sandboxes.as_ref(), &viewer, &task_id).await {
        Ok(scripts) => vec![success_exit(
            &id,
            json!({ "scripts": scripts::ui_script_list(&scripts) }),
        )],
        // A Thread the viewer cannot read runs no Scripts *for them*, and the
        // Run control sits in the header of a page they are already looking
        // at: `subtasks.list` answers the same condition the
        // same way, for the same reason.
        Err(ScriptsError::NotFound) => vec![success_exit(&id, json!({ "scripts": [] }))],
        Err(error) => vec![scripts_list_failure_exit(&id, &error, &identity)],
    }
}

pub(super) async fn sandbox_start(request: Request<SandboxStatusInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        scope,
        payload: SandboxStatusInput { thread_id: task_id },
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match sandbox::start(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_stop(request: Request<SandboxStatusInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        scope,
        payload: SandboxStatusInput { thread_id: task_id },
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match sandbox::stop(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_restart(request: Request<SandboxStatusInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        scope,
        payload: SandboxStatusInput { thread_id: task_id },
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match sandbox::restart(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_redeploy(request: Request<SandboxStatusInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        scope,
        payload: SandboxStatusInput { thread_id: task_id },
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match sandbox::redeploy(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_cleanup(request: Request<SandboxStatusInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        scope,
        payload: SandboxStatusInput { thread_id: task_id },
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match sandbox::cleanup(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_set_idle_timeout(
    request: Request<SandboxSetIdleTimeoutInput>,
) -> Vec<Value> {
    let Request {
        id,
        tag,
        scope,
        payload:
            SandboxSetIdleTimeoutInput {
                thread_id: task_id,
                minutes,
            },
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match sandbox::set_idle_timeout(state.commands.as_ref(), &viewer, &task_id, minutes).await {
        Ok(result) => vec![success_exit(&id, result)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_status(request: Request<SandboxStatusInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        scope,
        payload: SandboxStatusInput { thread_id: task_id },
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match sandbox::status(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_list_diagnostics(request: Request<IgnoredAny>) -> Vec<Value> {
    let Request {
        id,
        tag,
        scope,
        state,
        identity,
        ..
    } = request;
    match sandbox::list_diagnostics(&state.sandbox).await {
        Ok(entries) => vec![success_exit(&id, entries)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn subtasks_list(request: Request<SubtasksListInput>) -> Vec<Value> {
    let Request {
        id,
        payload: SubtasksListInput { thread_id: task_id },
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match subtasks::list(&state.subtasks, &viewer, &task_id).await {
        Ok(listing) => vec![success_exit(&id, listing)],
        // A Thread the viewer cannot read has no subtasks *for them*, and the
        // panel is a section on a page they are already looking at: an empty
        // list is the honest answer and a failure would red-flag a working page.
        Err(SubtasksError::NotFound) => vec![success_exit(&id, json!({ "subtasks": [] }))],
        Err(error) => vec![subtasks_failure_exit(&id, &error, &identity)],
    }
}
