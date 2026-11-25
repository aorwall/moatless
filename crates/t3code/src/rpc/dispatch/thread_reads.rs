//! Reads over one or a few named threads that no listing or projection
//! carries: find in a thread, and the titles links to other threads show.

use crate::authentication::viewer_scope;
use crate::oauth::Scope;
use crate::projection::FindStart;
use crate::projection::ThreadFind;
use crate::rpc::envelope::RequestId;
use crate::rpc::envelope::success_exit;
use crate::rpc::envelope::typed_failure_exit;
use crate::rpc::session::ProtocolSession;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use crate::snapshots::readable_under_a_project;
use crate::state::UiRpcState;
use crate::subscription::StreamFailure;
use crate::subscription::StreamSink;
use moatless_types::access_control::ScopeContext;
use serde::Deserialize;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;
use std::collections::HashSet;
use tracing::error;

/// How many Messages one find reads per query.
const FIND_PAGE: i64 = 200;

/// The most ids one title lookup answers for; the rest are left out, which
/// the contract reads as unknown.
const TITLE_LOOKUP_LIMIT: usize = 500;

pub(super) fn thread_read_methods() -> MethodTable {
    MethodTable::new()
        .detached(
            "orchestration.searchThread",
            Scope::OrchestrationRead,
            orchestration_search_thread,
        )
        .stream(
            "orchestration.searchThreadStream",
            Scope::OrchestrationRead,
            |session, request| Box::pin(session.orchestration_search_thread_stream(request)),
        )
        .detached(
            "orchestration.getThreadTitles",
            Scope::OrchestrationRead,
            orchestration_get_thread_titles,
        )
        .detached(
            "orchestration.getThreadLinkMetadata",
            Scope::OrchestrationRead,
            orchestration_get_thread_link_metadata,
        )
}

/// The contract's `OrchestrationV2SearchThreadInput`. `skills` is not read:
/// a `$skill` token is searched as written.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SearchThreadInput {
    thread_id: String,
    query: String,
    index: Option<u64>,
    offset: Option<i64>,
    start: Option<SearchThreadStart>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SearchThreadStart {
    entry_id: String,
    occurrence: u64,
}

/// The contract's `OrchestrationV2GetThreadTitlesInput`, which
/// `getThreadLinkMetadata` takes too.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadTitlesInput {
    thread_ids: Vec<String>,
}

fn search_error(id: &RequestId, cause: &str) -> Value {
    typed_failure_exit(
        id,
        json!({ "_tag": "OrchestrationV2SearchThreadError", "cause": cause }),
    )
}

/// Pages through the thread's Messages oldest first, sending the progressive
/// stream's first frame to `early` when there is one. The answer is the
/// final frame, or the failure frame to end the request with.
async fn find_in_thread(
    id: &RequestId,
    state: &UiRpcState,
    viewer: &ScopeContext,
    input: SearchThreadInput,
    early: Option<&StreamSink>,
) -> Result<Value, Value> {
    let task_id = input.thread_id.as_str();
    let conversations = state.reads.conversations();
    let read_failed = |error: sqlx::Error| {
        error!(%error, %task_id, "T3 UI thread find failed");
        search_error(id, "Failed to read the conversation.")
    };
    let head = conversations
        .conversation_head(task_id)
        .await
        .map_err(read_failed)?;
    if readable_under_a_project(&state.reads, viewer, task_id)
        .await
        .map_err(read_failed)?
        .is_none()
    {
        return Err(search_error(id, "Conversation not found"));
    }
    let sequence = u64::try_from(head.sequence()).unwrap_or(0);
    let start = input.start.map(|start| FindStart {
        entry_id: start.entry_id,
        occurrence: start.occurrence,
    });
    let mut find = ThreadFind::new(
        task_id,
        input.query.trim(),
        input.index,
        input.offset,
        start,
    );
    let mut after = None;
    loop {
        let page = conversations
            .messages_after(task_id, after, FIND_PAGE)
            .await
            .map_err(read_failed)?;
        for entry in &page {
            if let Some(frame) = find.feed(entry, sequence) {
                if let Some(sink) = early {
                    sink.send(frame).await;
                }
            }
        }
        if i64::try_from(page.len()).unwrap_or(0) < FIND_PAGE {
            break;
        }
        after = page.last().map(|entry| entry.id);
    }
    Ok(find.finish(sequence))
}

pub(super) async fn orchestration_search_thread(request: Request<SearchThreadInput>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match find_in_thread(&id, &state, &viewer, payload, None).await {
        Ok(result) => vec![success_exit(&id, result)],
        Err(failure) => vec![failure],
    }
}

impl ProtocolSession {
    /// `orchestration.searchThreadStream`: the first occurrence from `start`
    /// as soon as it is found, then the whole count, then the end.
    pub(super) async fn orchestration_search_thread_stream(
        &mut self,
        request: Request<SearchThreadInput>,
    ) -> Vec<Value> {
        let Request {
            id,
            payload,
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        self.streams
            .start_produced(&id, None, move |sink| async move {
                let id = sink.request_id().clone();
                match find_in_thread(&id, &state, &viewer, payload, Some(&sink)).await {
                    Ok(result) => {
                        sink.send(result).await;
                        Ok(())
                    }
                    Err(failure) => Err(StreamFailure::frame(failure)),
                }
            })
    }
}

/// Each requested thread the viewer may read, deleted and unreadable ones
/// left out, with its title and the project its link shows the icon of.
async fn thread_link_metadata(
    state: &UiRpcState,
    viewer: &ScopeContext,
    thread_ids: &[String],
) -> Result<Vec<(String, String, Option<String>)>, sqlx::Error> {
    let mut wanted: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for thread_id in thread_ids {
        if wanted.len() == TITLE_LOOKUP_LIMIT {
            break;
        }
        if seen.insert(thread_id.as_str()) {
            wanted.push(thread_id.clone());
        }
    }
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let conversations = state
        .reads
        .conversations()
        .readable_by_ids(viewer, &wanted)
        .await?;
    let referenced: Vec<String> = conversations
        .iter()
        .filter_map(|conversation| conversation.workspace_id.clone())
        .collect();
    let workspaces = state
        .reads
        .workspaces()
        .list_readable_including(viewer, &referenced)
        .await?;
    let listed: HashSet<&str> = workspaces
        .iter()
        .map(|listed| listed.workspace.id.as_str())
        .collect();
    Ok(conversations
        .iter()
        .map(|conversation| {
            // As `conversation_row` titles it.
            let title = conversation
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or("New thread")
                .to_string();
            let project_id = conversation
                .workspace_id
                .clone()
                .filter(|id| listed.contains(id.as_str()));
            (conversation.task_id.clone(), title, project_id)
        })
        .collect())
}

fn lookup_error(id: &RequestId, error: &sqlx::Error, viewer: &ScopeContext) -> Value {
    error!(%error, user_id = %viewer.user_id, "T3 UI thread title lookup failed");
    typed_failure_exit(
        id,
        json!({
            "_tag": "OrchestrationV2GetShellSnapshotError",
            "message": "Failed to read thread titles.",
        }),
    )
}

pub(super) async fn orchestration_get_thread_titles(
    request: Request<ThreadTitlesInput>,
) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match thread_link_metadata(&state, &viewer, &payload.thread_ids).await {
        Ok(threads) => {
            let titles: Map<String, Value> = threads
                .into_iter()
                .map(|(thread_id, title, _)| (thread_id, Value::String(title)))
                .collect();
            vec![success_exit(&id, Value::Object(titles))]
        }
        Err(error) => vec![lookup_error(&id, &error, &viewer)],
    }
}

pub(super) async fn orchestration_get_thread_link_metadata(
    request: Request<ThreadTitlesInput>,
) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match thread_link_metadata(&state, &viewer, &payload.thread_ids).await {
        Ok(threads) => {
            let metadata: Map<String, Value> = threads
                .into_iter()
                .map(|(thread_id, title, project_id)| {
                    (
                        thread_id,
                        json!({ "title": title, "projectId": project_id }),
                    )
                })
                .collect();
            vec![success_exit(&id, Value::Object(metadata))]
        }
        Err(error) => vec![lookup_error(&id, &error, &viewer)],
    }
}
