//! The orchestration V2 methods a V2 connection is answered with: see
//! [`OrchestrationProtocol`](crate::config::OrchestrationProtocol).
//!
//! Every read is built from [`thread_detail_v2`], and every write is a V1
//! command run through [`commands::dispatch`]; nothing here reads or writes
//! state of its own.

use super::routing::Request;
use super::routing::ThreadRequest;
use crate::authentication::viewer_scope;
use crate::commands;
use crate::commands::CommandContext;
use crate::config::OrchestrationProtocol;
use crate::rpc::frames::RequestId;
use crate::rpc::frames::failure_exit;
use crate::rpc::frames::shell_failure_exit;
use crate::rpc::frames::success_exit;
use crate::rpc::frames::typed_failure_exit;
use crate::rpc::session::ProtocolSession;
use crate::rpc::watch;
use crate::snapshots::after_sequence;
use crate::snapshots::archived_shell_page;
use crate::snapshots::thread_detail_v2;
use crate::state::UiRpcState;
use moatless_types::access_control::ScopeContext;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde_json::Value;
use serde_json::json;
use tracing::error;
use tracing::warn;
use uuid::Uuid;

pub(super) fn command_context<'a>(
    state: &'a UiRpcState,
    identity: &UiRpcAuthIdentity,
) -> CommandContext<'a> {
    CommandContext {
        conversations: state.reads.conversations(),
        agents: state.agents.as_ref(),
        sink: state.commands.as_ref(),
        workspace_script_commands: state.workspace_script_commands.as_ref(),
        skills: state.skills.as_ref(),
        caller: identity.user_id,
    }
}

/// `orchestration.dispatchCommand` for a V2 client. Refusals are the
/// contract's `OrchestrationV2DispatchCommandError`, which the composer renders.
pub(super) async fn orchestration_dispatch_command_v2(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let ctx = command_context(&state, &identity);
    match crate::v2_commands::dispatch(&ctx, &viewer_scope(&identity), &payload).await {
        Ok(result) => vec![success_exit(&id, result)],
        Err(refusal) => {
            let command = payload
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            warn!(
                %tag,
                user_id = %identity.user_id,
                command,
                reason = %refusal.message(),
                "T3 UI command refused"
            );
            vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": "OrchestrationV2DispatchCommandError",
                    "commandId": payload.get("commandId").cloned().unwrap_or_else(|| json!("")),
                    "commandType": payload.get("type").cloned().unwrap_or_else(|| json!("")),
                    "message": refusal.message(),
                }),
            )]
        }
    }
}

/// `orchestration.launchThread`: a thread and its first message in one call,
/// answered with the projection it opens on. `resumed` is always `false`: it
/// reports a retried launch, and Moatless keeps no launch receipts to tell one.
pub(super) async fn orchestration_launch_thread(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let launch_error = |message: String| {
        typed_failure_exit(
            &id,
            json!({
                "_tag": "OrchestrationV2ThreadLaunchError",
                "commandId": payload.get("commandId").cloned().unwrap_or_else(|| json!("")),
                "projectId": payload.get("projectId").cloned().unwrap_or_else(|| json!("")),
                "message": message,
            }),
        )
    };
    let requested = payload.get("threadId").and_then(Value::as_str);
    let reuse = payload
        .get("reuseExistingThread")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if reuse && requested.is_none() {
        return vec![launch_error(
            "Reusing an existing thread requires a thread id.".to_string(),
        )];
    }
    let thread_id = requested.map_or_else(|| Uuid::new_v4().to_string(), str::to_string);
    let ctx = command_context(&state, &identity);
    let dispatched = match launch_command(&payload, &thread_id) {
        Some(command) => commands::dispatch(&ctx, &viewer, &command).await.map(drop),
        None => Ok(()),
    };
    if let Err(refusal) = dispatched {
        warn!(
            user_id = %identity.user_id,
            reason = %refusal.message(),
            "T3 UI thread launch refused"
        );
        return vec![launch_error(refusal.message())];
    }
    match thread_detail_v2(
        &state.reads,
        &state.deployment(),
        state.subtasks.relations.as_ref(),
        &viewer,
        &thread_id,
    )
    .await
    {
        Ok(Some(detail)) => vec![success_exit(
            &id,
            json!({
                "threadId": thread_id,
                "projection": detail.snapshot["projection"],
                "resumed": false,
            }),
        )],
        Ok(None) => vec![launch_error(
            "The thread started but cannot be opened from here.".to_string(),
        )],
        Err(error) => {
            error!(%error, user_id = %identity.user_id, task_id = %thread_id, "Failed to assemble a launched T3 UI thread");
            vec![launch_error(
                "The thread started but could not be read.".to_string(),
            )]
        }
    }
}

/// The V1 command a launch is carried out as: a bootstrapped
/// `thread.turn.start`, or `thread.create` without a first message. `None` when
/// an existing thread is reused with nothing to send. Of the workspace strategy
/// only `branch` is read: a Task's checkout lives in its Sandbox.
pub(crate) fn launch_command(payload: &Value, thread_id: &str) -> Option<Value> {
    let reuse = payload
        .get("reuseExistingThread")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let branch = payload
        .get("workspaceStrategy")
        .and_then(|strategy| strategy.get("branch"))
        .cloned()
        .unwrap_or(Value::Null);
    let Some(initial) = payload.get("initialMessage") else {
        if reuse {
            return None;
        }
        return Some(json!({
            "type": "thread.create",
            "threadId": thread_id,
            "projectId": payload.get("projectId"),
            "modelSelection": payload.get("modelSelection"),
            "branch": branch,
        }));
    };
    let message_id = initial
        .get("messageId")
        .and_then(Value::as_str)
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_string);
    let mut command = json!({
        "type": "thread.turn.start",
        "threadId": thread_id,
        "modelSelection": payload.get("modelSelection"),
        "interactionMode": payload.get("interactionMode"),
        "message": {
            "messageId": message_id,
            "text": initial.get("text").cloned().unwrap_or_else(|| json!("")),
            "attachments": initial.get("attachments").cloned().unwrap_or_else(|| json!([])),
        },
    });
    if !reuse {
        command["bootstrap"] = json!({
            "createThread": {
                "projectId": payload.get("projectId"),
                "modelSelection": payload.get("modelSelection"),
                "branch": branch,
            }
        });
    }
    Some(command)
}

fn projection_error(id: &RequestId, task_id: &str, message: &str) -> Value {
    typed_failure_exit(
        id,
        json!({
            "_tag": "OrchestrationV2GetThreadProjectionError",
            "threadId": task_id,
            "message": message,
        }),
    )
}

async fn read_projection(request: ThreadRequest) -> Result<(RequestId, String, Value), Vec<Value>> {
    let ThreadRequest {
        id,
        viewer,
        task_id,
        state,
        identity,
        ..
    } = request;
    match thread_detail_v2(
        &state.reads,
        &state.deployment(),
        state.subtasks.relations.as_ref(),
        &viewer,
        &task_id,
    )
    .await
    {
        Ok(Some(mut detail)) => Ok((id, task_id, detail.snapshot["projection"].take())),
        Ok(None) => Err(vec![projection_error(
            &id,
            &task_id,
            "Conversation not found",
        )]),
        Err(error) => {
            error!(%error, user_id = %identity.user_id, %task_id, "Failed to assemble T3 UI thread projection");
            Err(vec![projection_error(
                &id,
                &task_id,
                "Failed to assemble the conversation",
            )])
        }
    }
}

pub(super) async fn orchestration_get_thread_projection(request: Request) -> Vec<Value> {
    let request = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
    match read_projection(request).await {
        Ok((id, _, projection)) => vec![success_exit(&id, projection)],
        Err(failure) => failure,
    }
}

/// One turn item, by id. Items here are never withheld from the projection,
/// so this answers with the same row the timeline already holds.
pub(super) async fn orchestration_get_turn_item(request: Request) -> Vec<Value> {
    let request = match request.require_thread_id() {
        Ok(request) => request,
        Err(failure) => return failure,
    };
    let item_id = request
        .payload
        .get("itemId")
        .and_then(Value::as_str)
        .map(str::to_string);
    match read_projection(request).await {
        Ok((id, _, projection)) => {
            let item = projection["turnItems"]
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.get("id").and_then(Value::as_str) == item_id.as_deref())
                })
                .cloned()
                .unwrap_or(Value::Null);
            vec![success_exit(&id, json!({ "item": item }))]
        }
        Err(failure) => failure,
    }
}

impl ProtocolSession {
    /// `orchestration.subscribeThread` for a V2 client: the projection whole,
    /// then whole again each time the conversation moves. The client replaces
    /// what it holds on every `snapshot`, so no event vocabulary is needed.
    pub(super) async fn orchestration_subscribe_thread_v2(
        &mut self,
        request: Request,
    ) -> Vec<Value> {
        let ThreadRequest {
            id,
            viewer,
            task_id,
            state,
            identity,
            payload,
            ..
        } = match request.require_thread_id() {
            Ok(request) => request,
            Err(failure) => return failure,
        };
        let detail = match thread_detail_v2(
            &state.reads,
            &state.deployment(),
            state.subtasks.relations.as_ref(),
            &viewer,
            &task_id,
        )
        .await
        {
            Ok(Some(detail)) => detail,
            Ok(None) => return vec![projection_error(&id, &task_id, "Conversation not found")],
            Err(error) => {
                error!(%error, user_id = %identity.user_id, %task_id, "Failed to assemble T3 UI thread snapshot");
                return vec![failure_exit(&id, "Failed to assemble the conversation")];
            }
        };
        let mut values = Vec::new();
        // A client already at this head holds what a snapshot would send.
        let behind = after_sequence(&payload).is_none_or(|after| after < detail.head.sequence());
        if behind {
            values.push(watch::thread_snapshot_item(&detail));
        }
        if payload
            .get("requestCompletionMarker")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            values.push(json!({ "kind": "synchronized" }));
        }
        let watched = watch::WatchedThreadV2 {
            reads: state.reads.clone(),
            lineage: detail.lineage,
            relations: state.subtasks.relations.clone(),
            frontend_url: state.frontend_url.clone(),
            model_defaults: state.model_defaults.clone(),
            viewer,
            task_id,
            head: detail.head,
        };
        self.streams.start_produced(&id, Some(values), move |sink| {
            watch::watch_thread_v2(sink, watched)
        })
    }

    /// `orchestration.subscribeArchivedShell`: the newest archived page, then
    /// that page again whenever the listing moves.
    pub(super) async fn orchestration_subscribe_archived_shell(
        &mut self,
        request: Request,
    ) -> Vec<Value> {
        let Request {
            id,
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        let snapshot = match archived_snapshot(&state, &viewer).await {
            Ok(snapshot) => snapshot,
            Err(error) => return vec![shell_failure_exit(&id, &error, &identity)],
        };
        let reads = state.reads.clone();
        let model_defaults = state.model_defaults.clone();
        let initial = vec![json!({ "kind": "snapshot", "snapshot": snapshot.clone() })];
        self.streams
            .start_produced(&id, Some(initial), move |sink| {
                watch::watch_archived_shell(sink, reads, model_defaults, viewer, snapshot)
            })
    }
}

async fn archived_snapshot(
    state: &UiRpcState,
    viewer: &ScopeContext,
) -> Result<Value, sqlx::Error> {
    archived_shell_page(
        &state.reads,
        &state.model_defaults,
        viewer,
        None,
        OrchestrationProtocol::V2,
    )
    .await
}

#[cfg(test)]
mod tests;
