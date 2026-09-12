//! Methods that read or command the thread listing.

use super::routing::Request;
use crate::authentication::viewer_scope;
use crate::commands;
use crate::dao::ConversationBrowse;
use crate::projection;
use crate::rpc::frames::failure_exit;
use crate::rpc::frames::shell_failure_exit;
use crate::rpc::frames::success_exit;
use crate::rpc::frames::typed_failure_exit;
use crate::rpc::session::SEARCH_THREADS_LIMIT;
use crate::rpc::session::SEARCH_THREADS_MIN_QUERY;
use crate::snapshots::archived_shell_snapshot;
use crate::snapshots::browse_snapshot;
use crate::snapshots::thread_shell;
use serde_json::Value;
use serde_json::json;
use tracing::error;
use tracing::warn;

/// The one write. A refusal is a typed failure the composer renders, not a
/// defect: "that action does not exist here" is true, "this server is broken" is
/// not.
pub(super) async fn orchestration_dispatch_command(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let ctx = commands::CommandContext {
        conversations: state.reads.conversations(),
        agents: state.agents.as_ref(),
        sink: state.commands.as_ref(),
        workspace_script_commands: state.workspace_script_commands.as_ref(),
        skills: state.skills.as_ref(),
        caller: identity.user_id,
    };
    match commands::dispatch(&ctx, &viewer_scope(&identity), &payload).await {
        Ok(result) => vec![success_exit(&id, result)],
        Err(refusal) => {
            // `warn!`, not `debug!`: this is the surface's only write, and at
            // `RUST_LOG=info` a refused one would otherwise leave nothing behind
            // at all.
            warn!(
                %tag,
                user_id = %identity.user_id,
                reason = %refusal.message(),
                "T3 UI command refused"
            );
            vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": "OrchestrationDispatchCommandError",
                    "message": refusal.message(),
                }),
            )]
        }
    }
}

pub(super) async fn orchestration_search_threads(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .map_or("", str::trim);
    // See [`SEARCH_THREADS_MIN_QUERY`]. Answering an empty list is the honest
    // response to "match anything", and it costs nothing.
    if query.chars().count() < SEARCH_THREADS_MIN_QUERY {
        return vec![success_exit(&id, json!({ "matches": [] }))];
    }
    let limit = payload
        .get("limit")
        .and_then(Value::as_i64)
        .filter(|limit| *limit > 0)
        .map_or(SEARCH_THREADS_LIMIT, |limit| {
            limit.min(SEARCH_THREADS_LIMIT)
        });
    match state
        .reads
        .conversations()
        .search_conversations(&viewer_scope(&identity), query, limit)
        .await
    {
        Ok(matches) => vec![success_exit(
            &id,
            json!({ "matches": projection::thread_matches(&matches, query) }),
        )],
        Err(error) => {
            error!(%error, user_id = %identity.user_id, "T3 UI thread search failed");
            vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": "OrchestrationSearchThreadsError",
                    "message": "Failed to search conversations.",
                }),
            )]
        }
    }
}

/// Still answers with nothing, and deliberately. This is not the path a
/// reconnecting client takes: no T3 client calls it — web, desktop and mobile
/// all share `client-runtime`, whose only resume is `subscribeShell` with
/// `afterSequence` (`state/shell.ts:181`), and `replayEvents` appears outside
/// `packages/contracts` only in the reference server and its own tests. Its
/// result type is `OrchestrationEvent`, which carries an event id, an aggregate,
/// a causation chain and a typed payload; a listing change record carries a
/// position and a Task id and nothing else, on purpose. Filling this in would
/// mean inventing an event log this backend does not have, for a method nothing
/// calls.
pub(super) fn orchestration_replay_events(request: Request) -> Vec<Value> {
    vec![success_exit(&request.id, json!([]))]
}

pub(super) async fn orchestration_get_archived_shell_snapshot(request: Request) -> Vec<Value> {
    let Request {
        id,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match archived_shell_snapshot(&state.reads, &state.model_defaults, &viewer).await {
        Ok(snapshot) => vec![success_exit(&id, snapshot)],
        Err(error) => vec![shell_failure_exit(&id, &error, &identity)],
    }
}

/// The listings' complement: one row, by id, for a conversation the client is
/// holding and no listing named. `null` is the answer for every way it can be
/// absent — the client is already on that page, and a failure would take a
/// rendered transcript off the screen to report a missing title.
pub(super) async fn threads_get_shell(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(&id, "threads.getShell requires a threadId")];
    };
    match thread_shell(&state.reads, &state.model_defaults, &viewer, task_id).await {
        Ok(thread) => vec![success_exit(
            &id,
            json!({ "thread": thread.unwrap_or(Value::Null) }),
        )],
        Err(error) => vec![shell_failure_exit(&id, &error, &identity)],
    }
}

/// The listing rows a filter names, for conversations outside the viewer's own
/// listing.
///
/// The refusal is the filter being empty, not the viewer not being an
/// administrator: what a browse may reach is the read rule, which
/// `list_browsable` applies to everyone alike. Empty is refused because that
/// read unnarrowed is every Task an administrator may read.
pub(super) async fn threads_browse(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_owned);
    let Some(filter) = ConversationBrowse::new(
        text("ownerUserId"),
        text("tag"),
        payload
            .get("includeClosed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ) else {
        warn!(
            user_id = %viewer.user_id,
            "T3 UI thread browse refused; it names neither an owner nor a tag"
        );
        return vec![failure_exit(
            &id,
            "threads.browse requires an ownerUserId or a tag",
        )];
    };
    match browse_snapshot(&state.reads, &state.model_defaults, &viewer, &filter).await {
        Ok(threads) => vec![success_exit(&id, threads)],
        Err(error) => vec![shell_failure_exit(&id, &error, &identity)],
    }
}
