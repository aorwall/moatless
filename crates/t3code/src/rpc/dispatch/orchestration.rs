//! Methods that read or command the thread listing.

use crate::authentication::viewer_scope;
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
use crate::snapshots::archived_shell_page;
use crate::snapshots::browse_snapshot;
use crate::snapshots::thread_shell;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tracing::error;
use tracing::warn;

/// The thread listing's reads.
pub(super) fn orchestration_methods() -> MethodTable {
    MethodTable::new()
        .detached(
            "orchestration.getArchivedShellSnapshot",
            Scope::OrchestrationRead,
            orchestration_get_archived_shell_snapshot,
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

/// The contract's `OrchestrationSearchThreadsInput`.
#[derive(Deserialize)]
pub(super) struct OrchestrationSearchThreadsInput {
    query: Option<String>,
    limit: Option<i64>,
}

/// The contract's `orchestration.getArchivedShellSnapshot` input.
#[derive(Deserialize)]
pub(super) struct OrchestrationGetArchivedShellSnapshotInput {
    before: Option<String>,
}

/// The contract's `ThreadShellGetInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadShellGetInput {
    thread_id: String,
}

/// The contract's `ThreadBrowseInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadBrowseInput {
    owner_user_id: Option<String>,
    tag: Option<String>,
    include_closed: Option<bool>,
}

pub(super) async fn orchestration_search_threads(
    request: Request<OrchestrationSearchThreadsInput>,
) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let query = payload.query.as_deref().map_or("", str::trim);
    // See [`SEARCH_THREADS_MIN_QUERY`]. Answering an empty list is the honest
    // response to "match anything", and it costs nothing.
    if query.chars().count() < SEARCH_THREADS_MIN_QUERY {
        return vec![success_exit(&id, json!({ "matches": [] }))];
    }
    let limit = payload
        .limit
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
            // A timeout is the search's own bound shedding a slow term, counted
            // by `moatless_task_search_timeouts_total`; at ERROR one typeahead
            // session trips the error-burst alert.
            if moatless_core::db::is_statement_timeout(&error) {
                warn!(%error, user_id = %identity.user_id, "T3 UI thread search cancelled by its statement timeout");
            } else {
                error!(%error, user_id = %identity.user_id, "T3 UI thread search failed");
            }
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

/// `before` is optional: a client that sends none gets the newest page.
pub(super) async fn orchestration_get_archived_shell_snapshot(
    request: Request<OrchestrationGetArchivedShellSnapshotInput>,
) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let before = match payload.before.as_deref() {
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
    match archived_shell_page(&state.reads, &state.model_defaults, &viewer, before).await {
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
pub(super) async fn threads_get_shell(request: Request<ThreadShellGetInput>) -> Vec<Value> {
    let Request {
        id,
        payload: ThreadShellGetInput { thread_id: task_id },
        state,
        identity,
        ..
    } = request;
    match thread_shell(
        &state.reads,
        &state.model_defaults,
        &viewer_scope(&identity),
        &task_id,
    )
    .await
    {
        Ok(thread) => vec![success_exit(
            &id,
            json!({ "thread": thread.unwrap_or(Value::Null) }),
        )],
        Err(error) => vec![shell_failure_exit(&id, &error.into(), &identity)],
    }
}

/// The listing rows a filter names, for conversations outside the viewer's own
/// listing.
///
/// Refused when the filter is empty, not when the viewer isn't an
/// administrator: `list_browsable` applies the read rule to everyone alike, and
/// unnarrowed that read is every Task an administrator may read.
pub(super) async fn threads_browse(request: Request<ThreadBrowseInput>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(filter) = ConversationBrowse::new(
        payload.owner_user_id,
        payload.tag,
        payload.include_closed.unwrap_or(false),
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
