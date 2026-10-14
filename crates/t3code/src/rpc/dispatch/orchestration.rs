//! Methods that read or command the thread listing.

use crate::authentication::viewer_scope;
use crate::commands;
use crate::dao::ArchiveCursor;
use crate::dao::ConversationBrowse;
use crate::oauth::Scope;
use crate::projection;
use crate::rpc::envelope::failure_exit;
use crate::rpc::envelope::success_exit;
use crate::rpc::envelope::typed_failure_exit;
use crate::rpc::errors::shell_failure_exit;
use crate::rpc::session::SEARCH_THREADS_LIMIT;
use crate::rpc::session::SEARCH_THREADS_MIN_QUERY;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use crate::rpc::table::ThreadRequest;
use crate::snapshots::archived_shell_page;
use crate::snapshots::browse_snapshot;
use crate::snapshots::thread_shell;
use serde_json::Value;
use serde_json::json;
use tracing::error;
use tracing::warn;

/// The thread listing's reads and the V1 command path.
pub(super) fn orchestration_methods() -> MethodTable {
    MethodTable::new()
        .session(
            "orchestration.dispatchCommand",
            Scope::OrchestrationOperate,
            orchestration_dispatch_command,
        )
        .detached(
            "orchestration.getArchivedShellSnapshot",
            Scope::OrchestrationRead,
            orchestration_get_archived_shell_snapshot,
        )
        .session(
            "orchestration.replayEvents",
            Scope::OrchestrationRead,
            orchestration_replay_events,
        )
        .detached(
            "orchestration.searchThreads",
            Scope::OrchestrationRead,
            orchestration_search_threads,
        )
        .detached("threads.browse", Scope::OrchestrationRead, threads_browse)
        .detached(
            "threads.getShell",
            Scope::OrchestrationRead,
            threads_get_shell,
        )
}

/// The one write. A refusal is a typed failure the composer renders, not a
/// defect: "that action does not exist here" is true, "this server is broken"
/// is not.
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

/// Answers with nothing, deliberately: no T3 client calls it, since all three
/// share `client-runtime`, whose only resume is `subscribeShell` with
/// `afterSequence` (`state/shell.ts:181`).
///
/// Its `OrchestrationEvent` result carries an event id, an aggregate and a
/// causation chain, none of which this backend records.
pub(super) async fn orchestration_replay_events(request: Request) -> Vec<Value> {
    vec![success_exit(&request.id, json!([]))]
}

/// `before` is optional: a client that sends none gets the newest page.
pub(super) async fn orchestration_get_archived_shell_snapshot(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        shared,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let before = match payload.get("before").and_then(Value::as_str) {
        None => None,
        Some(text) => match ArchiveCursor::decode(text) {
            Some(cursor) => Some(cursor),
            None => {
                return vec![failure_exit(
                    &id,
                    "orchestration.getArchivedShellSnapshot: `before` is not a cursor it returned",
                )];
            }
        },
    };
    match archived_shell_page(
        &state.reads,
        &state.model_defaults,
        &viewer,
        before,
        shared.protocol,
    )
    .await
    {
        Ok(snapshot) => vec![success_exit(&id, snapshot)],
        Err(error) => vec![shell_failure_exit(&id, &error, &identity)],
    }
}

/// The listings' complement: one row, by id, for a conversation the client is
/// holding and no listing named.
///
/// `null` answers every way it can be absent: the client is already on that
/// page, and a failure would take a rendered transcript off screen to report a
/// missing title.
pub(super) async fn threads_get_shell(request: Request) -> Vec<Value> {
    let protocol = request.shared.protocol;
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
    match thread_shell(
        &state.reads,
        &state.model_defaults,
        &viewer,
        &task_id,
        protocol,
    )
    .await
    {
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
/// Refused when the filter is empty, not when the viewer isn't an
/// administrator: `list_browsable` applies the read rule to everyone alike, and
/// unnarrowed that read is every Task an administrator may read.
pub(super) async fn threads_browse(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        shared,
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
    match browse_snapshot(
        &state.reads,
        &state.model_defaults,
        &viewer,
        &filter,
        shared.protocol,
    )
    .await
    {
        Ok(threads) => vec![success_exit(&id, threads)],
        Err(error) => vec![shell_failure_exit(&id, &error, &identity)],
    }
}
