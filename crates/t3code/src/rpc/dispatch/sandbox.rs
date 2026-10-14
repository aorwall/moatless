//! Methods that act on a thread's Sandbox and read what runs inside it.

use crate::authentication::viewer_scope;
use crate::oauth::Scope;
use crate::rpc::envelope::failure_exit;
use crate::rpc::envelope::success_exit;
use crate::rpc::errors::sandbox_failure_exit;
use crate::rpc::errors::scripts_failure_exit;
use crate::rpc::errors::scripts_list_failure_exit;
use crate::rpc::errors::subtasks_failure_exit;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use crate::rpc::table::ThreadRequest;
use crate::sandbox;
use crate::scripts;
use crate::scripts::ScriptsError;
use crate::subtasks;
use crate::subtasks::SubtasksError;
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

pub(super) async fn scripts_run(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match scripts::run(
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
        Err(error) => vec![scripts_failure_exit(&id, &error, &payload, &identity)],
    }
}

pub(super) async fn scripts_list_for_thread(request: Request) -> Vec<Value> {
    let ThreadRequest {
        id,
        viewer,
        task_id,
        state,
        identity,
        ..
    } = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
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

pub(super) async fn sandbox_start(request: Request) -> Vec<Value> {
    let ThreadRequest {
        id,
        tag,
        scope,
        viewer,
        task_id,
        state,
        identity,
        ..
    } = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
    match sandbox::start(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_stop(request: Request) -> Vec<Value> {
    let ThreadRequest {
        id,
        tag,
        scope,
        viewer,
        task_id,
        state,
        identity,
        ..
    } = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
    match sandbox::stop(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_restart(request: Request) -> Vec<Value> {
    let ThreadRequest {
        id,
        tag,
        scope,
        viewer,
        task_id,
        state,
        identity,
        ..
    } = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
    match sandbox::restart(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_redeploy(request: Request) -> Vec<Value> {
    let ThreadRequest {
        id,
        tag,
        scope,
        viewer,
        task_id,
        state,
        identity,
        ..
    } = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
    match sandbox::redeploy(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_cleanup(request: Request) -> Vec<Value> {
    let ThreadRequest {
        id,
        tag,
        scope,
        viewer,
        task_id,
        state,
        identity,
        ..
    } = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
    match sandbox::cleanup(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_set_idle_timeout(request: Request) -> Vec<Value> {
    let ThreadRequest {
        id,
        tag,
        scope,
        viewer,
        task_id,
        state,
        identity,
        payload,
    } = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
    // `as_i64` and not `as_u64`: a client that sends a negative gets the
    // refusal `sandbox::set_idle_timeout` states, not "requires minutes".
    let Some(minutes) = payload
        .get("minutes")
        .and_then(Value::as_i64)
        .and_then(|minutes| i32::try_from(minutes).ok())
    else {
        return vec![failure_exit(&id, "sandbox.setIdleTimeout requires minutes")];
    };
    match sandbox::set_idle_timeout(state.commands.as_ref(), &viewer, &task_id, minutes).await {
        Ok(result) => vec![success_exit(&id, result)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_status(request: Request) -> Vec<Value> {
    let ThreadRequest {
        id,
        tag,
        scope,
        viewer,
        task_id,
        state,
        identity,
        ..
    } = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
    match sandbox::status(&state.sandbox, &viewer, &task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
    }
}

pub(super) async fn sandbox_list_diagnostics(request: Request) -> Vec<Value> {
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

pub(super) async fn subtasks_list(request: Request) -> Vec<Value> {
    let ThreadRequest {
        id,
        viewer,
        task_id,
        state,
        identity,
        ..
    } = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
    match subtasks::list(&state.subtasks, &viewer, &task_id).await {
        Ok(listing) => vec![success_exit(&id, listing)],
        // A Thread the viewer cannot read has no subtasks *for them*, and the
        // panel is a section on a page they are already looking at: an empty
        // list is the honest answer and a failure would red-flag a working page.
        Err(SubtasksError::NotFound) => vec![success_exit(&id, json!({ "subtasks": [] }))],
        Err(error) => vec![subtasks_failure_exit(&id, &error, &identity)],
    }
}
