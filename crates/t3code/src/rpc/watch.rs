use crate::change_watch::ThreadSignal;
use crate::change_watch::next_change;
use crate::config::AgentModelDefaults;
use crate::config::OrchestrationProtocol;
use crate::dao::ConversationDao;
use crate::feed;
use crate::reads::T3Reads;
use crate::review;
use crate::rpc::metrics::THREAD_SUBSCRIPTIONS;
use crate::snapshots::ThreadDetailV2;
use crate::snapshots::archived_shell_page;
use crate::snapshots::thread_detail_v2_under;
use crate::state::Deployment;
use crate::subscription;
use crate::subscription::StreamFailure;
use crate::subscription::StreamResult;
use crate::subscriptions::SHELL_POLL_INTERVAL;
use crate::subscriptions::THREAD_ACCESS_RECHECK_INTERVAL;
use crate::subscriptions::THREAD_POLL_INTERVAL;
use crate::subscriptions::VCS_STATUS_POLL_INTERVAL;
use crate::subscriptions::poll_thread;
use feed::ShellFeed;
use feed::ThreadFeed;
use moatless_core::change_feed::conversation::ConversationHead;
use moatless_core::change_feed::listing::ListingHead;
use moatless_types::access_control::ScopeContext;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::warn;

/// Reports one conversation's branch state, polled rather than pushed since the
/// state lives in a Sandbox's checkout an agent writes to.
///
/// A pass that can't reach the Sandbox says nothing rather than ending the
/// stream, which would present a stopped sandbox as a permanent answer.
pub(crate) async fn watch_vcs_status(
    sink: subscription::StreamSink,
    cwd: String,
    conversations: Arc<dyn ConversationDao>,
    sandboxes: Arc<dyn sandbox_proto::client::SandboxConnector>,
    pull_requests: Arc<dyn crate::ports::BoundPullRequestSource>,
    viewer: ScopeContext,
) -> StreamResult {
    let mut last_sent: Option<Value> = None;
    loop {
        if let Ok(snapshot) = review::status(
            conversations.as_ref(),
            sandboxes.as_ref(),
            pull_requests.as_ref(),
            &viewer,
            &cwd,
        )
        .await
        {
            // Re-sending an unchanged snapshot costs the client a rerender
            // of every surface that reads git status, so only a change goes
            // on the wire.
            if last_sent.as_ref() != Some(&snapshot) {
                if !sink.send(snapshot.clone()).await {
                    return Ok(());
                }
                last_sent = Some(snapshot);
            }
        }
        tokio::time::sleep(VCS_STATUS_POLL_INTERVAL).await;
    }
}

/// Watches the listing feed until the socket goes away, delivering at most one
/// chunk per move of the listing head regardless of how many rows changed.
///
/// Chunks are not gated on the client's `Ack` — it acks as soon as it has
/// queued the chunk — so [`ChangePoller`](crate::change_watch::ChangePoller)'s
/// interval alone bounds this to four chunks a second.
pub(crate) async fn watch_shell(
    sink: subscription::StreamSink,
    mut feed: ShellFeed,
    reads: T3Reads,
    model_defaults: Arc<crate::config::AgentModelDefaults>,
    viewer: ScopeContext,
    mut listing: watch::Receiver<Option<ListingHead>>,
) -> StreamResult {
    // What the feed holds, so a subscription opened at the published head
    // reads nothing until the head moves.
    let mut seen = Some(feed.read_through());
    loop {
        let Some(head) = next_change(&mut listing, seen.as_ref(), |_| true).await else {
            return Ok(());
        };
        let values = match feed.poll_at(&head, &reads, &model_defaults, &viewer).await {
            Ok(values) => values,
            Err(error) => {
                // The subscription survives a failed poll rather than ending
                // as a dead sidebar; neither the cursor nor `seen` has moved,
                // so the next pass retries the same ground. That holds because
                // [`ShellFeed::poll_at`] advances the cursor last, after every
                // read — do not move it back here.
                warn!(
                    %error,
                    user_id = %viewer.user_id,
                    "Failed to read the T3 UI shell change feed"
                );
                tokio::time::sleep(SHELL_POLL_INTERVAL).await;
                continue;
            }
        };
        // A filled batch leaves the cursor short of `head`. Remembering the
        // cursor instead makes the next pass read the rest one interval from
        // now, rather than at the next listing move, which may never come.
        let through = feed.read_through();
        let behind = through.identity == head.identity && through.position < head.position;
        seen = Some(if behind { through } else { head });
        // Left where the poll put it: a closed channel means the socket
        // loop (held by `ProtocolSession`) already returned, so no next
        // poll exists to serve a rewound cursor — the client resumes via
        // the snapshot's `afterSequence` on resubscribe instead. Queuing a
        // chunk isn't delivery: it can still fail later in `send_frames`.
        if !values.is_empty() && !sink.send_values(values).await {
            return Ok(());
        }
        if behind {
            tokio::time::sleep(SHELL_POLL_INTERVAL).await;
        }
    }
}

/// Mirrors [`watch_shell`] over the conversation's own change log, polling
/// when the conversation or its listing row moves.
///
/// A poll can also answer `None` — the conversation is no longer readable
/// (deleted, unfollowed, ungrouped) — and the subscription ends rather than
/// idling on a stale snapshot.
pub(crate) async fn watch_thread(
    sink: subscription::StreamSink,
    mut feed: ThreadFeed,
    reads: T3Reads,
    frontend_url: Arc<str>,
    model_defaults: Arc<AgentModelDefaults>,
    viewer: ScopeContext,
    mut signals: watch::Receiver<Option<ThreadSignal>>,
) -> StreamResult {
    let _open = OpenThreadSubscription::count();
    let deployment = Deployment {
        model_defaults: &model_defaults,
        frontend_url: &frontend_url,
    };
    let mut seen: Option<ThreadSignal> = None;
    loop {
        // Polled on the timeout too: access can end without a listing change,
        // as when a Workspace membership does.
        let changed = next_change(&mut signals, seen.as_ref(), |_| true);
        let signal = match tokio::time::timeout(THREAD_ACCESS_RECHECK_INTERVAL, changed).await {
            Ok(Some(signal)) => Some(signal),
            Ok(None) => return Ok(()),
            Err(_elapsed) => seen,
        };
        let polled = poll_thread(&mut feed, &reads, &deployment, &viewer).await;
        let values = match polled {
            Ok(Some(values)) => values,
            Ok(None) => {
                return Err(StreamFailure::message("Conversation not found"));
            }
            Err(error) => {
                // As in `watch_shell`: a failed poll leaves the cursor and
                // `seen` where they were, so the next one covers the same
                // ground and the subscription survives a transient database
                // error.
                warn!(
                    %error,
                    user_id = %viewer.user_id,
                    "Failed to poll the T3 UI conversation"
                );
                tokio::time::sleep(THREAD_POLL_INTERVAL).await;
                continue;
            }
        };
        // A filled batch leaves the feed short of the head it was woken for.
        // Forgetting `seen` makes the next pass read the rest one interval
        // from now, rather than at the next move or access recheck.
        let behind = signal.is_some_and(|signal| feed.is_behind(signal.head));
        seen = if behind { None } else { signal };
        if !values.is_empty() && !sink.send_values(values).await {
            return Ok(());
        }
        if behind {
            tokio::time::sleep(THREAD_POLL_INTERVAL).await;
        }
    }
}

/// What a V2 thread subscription re-reads on each pass.
pub(crate) struct WatchedThreadV2 {
    pub(crate) reads: T3Reads,
    /// Resolved when the subscription opened; a thread's ancestry never moves.
    pub(crate) lineage: crate::projection::Lineage,
    /// Re-read on each pass: the thread's delegated Tasks are its subagents.
    pub(crate) relations: Arc<dyn crate::ports::TaskRelationSource>,
    pub(crate) frontend_url: Arc<str>,
    pub(crate) model_defaults: Arc<AgentModelDefaults>,
    pub(crate) viewer: ScopeContext,
    pub(crate) task_id: String,
    /// The head the client was last sent, or already held.
    pub(crate) head: ConversationHead,
    pub(crate) signals: watch::Receiver<Option<ThreadSignal>>,
}

/// The `snapshot` stream item a V2 thread subscription sends.
///
/// Without the history keys: the client reads their absence as a whole
/// projection and resets its history state, which is what this is.
pub(crate) fn thread_snapshot_item(detail: &ThreadDetailV2) -> Value {
    json!({
        "kind": "snapshot",
        "snapshotSequence": detail.snapshot["snapshotSequence"],
        "projection": detail.snapshot["projection"],
    })
}

/// Mirrors [`watch_thread`], answering every move of the conversation's head
/// with the whole projection rather than the events between.
pub(crate) async fn watch_thread_v2(
    sink: subscription::StreamSink,
    mut watched: WatchedThreadV2,
) -> StreamResult {
    let _open = OpenThreadSubscription::count();
    let deployment = Deployment {
        model_defaults: &watched.model_defaults,
        frontend_url: &watched.frontend_url,
    };
    loop {
        let held = watched.head.position;
        let moved = next_change(&mut watched.signals, None, |signal| {
            signal.head.position > held
        });
        if moved.await.is_none() {
            return Ok(());
        }
        let detail = thread_detail_v2_under(
            &watched.reads,
            &deployment,
            watched.relations.as_ref(),
            &watched.viewer,
            &watched.task_id,
            watched.lineage.clone(),
        )
        .await;
        let detail = match detail {
            Ok(Some(detail)) => detail,
            Ok(None) => return Err(StreamFailure::message("Conversation not found")),
            Err(error) => {
                // As in `watch_thread`: the head is not advanced, so the next
                // pass rebuilds the same snapshot.
                warn!(%error, user_id = %watched.viewer.user_id, "Failed to assemble the T3 UI conversation");
                tokio::time::sleep(THREAD_POLL_INTERVAL).await;
                continue;
            }
        };
        watched.head = detail.head;
        if !sink.send(thread_snapshot_item(&detail)).await {
            return Ok(());
        }
    }
}

/// How often an open archive listing asks whether the listing moved. Longer
/// than the shell's: a hit re-reads a whole page, and the archive changes only
/// when a conversation is closed or reopened.
const ARCHIVED_SHELL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Re-sends the newest archived page whenever the listing head moves, starting
/// from `initial`, the page the client was already sent. Its `snapshotSequence`
/// is the listing position read before its rows, so no change is missed.
pub(crate) async fn watch_archived_shell(
    sink: subscription::StreamSink,
    reads: T3Reads,
    model_defaults: Arc<AgentModelDefaults>,
    viewer: ScopeContext,
    initial: Value,
    listing: watch::Receiver<Option<ListingHead>>,
) -> StreamResult {
    let mut last = initial["snapshotSequence"].as_i64();
    let mut sent = Some(initial["threads"].clone());
    loop {
        tokio::time::sleep(ARCHIVED_SHELL_POLL_INTERVAL).await;
        let Some(position) = listing.borrow().as_ref().map(|head| head.position) else {
            continue;
        };
        if last == Some(position) {
            continue;
        }
        let snapshot = match archived_shell_page(
            &reads,
            &model_defaults,
            &viewer,
            None,
            OrchestrationProtocol::V2,
        )
        .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                warn!(%error, user_id = %viewer.user_id, "Failed to read the T3 UI archive");
                continue;
            }
        };
        last = Some(position);
        // The sequence moves with every listing change; only the rows decide
        // whether the client has anything new to render.
        if sent.as_ref() == Some(&snapshot["threads"]) {
            continue;
        }
        sent = Some(snapshot["threads"].clone());
        if !sink
            .send(json!({ "kind": "snapshot", "snapshot": snapshot }))
            .await
        {
            return Ok(());
        }
    }
}

/// One open thread subscription on [`THREAD_SUBSCRIPTIONS`], released on drop
/// so a stream aborted mid-poll is uncounted too.
struct OpenThreadSubscription;

impl OpenThreadSubscription {
    fn count() -> Self {
        metrics::gauge!(THREAD_SUBSCRIPTIONS).increment(1.0);
        Self
    }
}

impl Drop for OpenThreadSubscription {
    fn drop(&mut self) {
        metrics::gauge!(THREAD_SUBSCRIPTIONS).decrement(1.0);
    }
}

/// Which Tasks one subscriber may read, answered once per Task per
/// subscription.
///
/// Every broadcast below is environment-wide and the client filters by thread
/// for display only, so a stream that drops this check has no second gate
/// behind it.
struct ReadableTasks {
    tasks: Arc<dyn moatless_ports::task::TaskReader>,
    viewer: ScopeContext,
    answered: HashMap<String, bool>,
}

impl ReadableTasks {
    fn new(tasks: Arc<dyn moatless_ports::task::TaskReader>, viewer: ScopeContext) -> Self {
        Self {
            tasks,
            viewer,
            answered: HashMap::new(),
        }
    }

    /// `None` when the database could not say, which the caller must treat as
    /// "do not send" rather than as a refusal to cache.
    async fn allows(&mut self, task_id: &str) -> Option<bool> {
        if let Some(answered) = self.answered.get(task_id) {
            return Some(*answered);
        }
        match self.tasks.may_access(task_id, &self.viewer).await {
            Ok(allowed) => {
                self.answered.insert(task_id.to_string(), allowed);
                Some(allowed)
            }
            Err(error) => {
                warn!(%error, task_id, "Could not authorize a stream event");
                None
            }
        }
    }
}

/// Every preview event the viewer is allowed to see, until the socket closes.
pub(crate) async fn watch_preview_events(
    sink: subscription::StreamSink,
    preview: crate::preview::PreviewState,
    viewer: ScopeContext,
) -> StreamResult {
    let mut events = preview.tabs.subscribe();
    let mut readable = ReadableTasks::new(preview.tasks.clone(), viewer.clone());
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            // Lagged: this subscriber fell behind and lost events. It is
            // not a reason to end the stream — the client reconciles
            // against `preview.list` on the next event it does see.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                warn!(missed, user_id = %viewer.user_id, "A preview event subscriber lagged");
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
        };
        let Some(task_id) = event.get("threadId").and_then(Value::as_str) else {
            continue;
        };
        if readable.allows(task_id).await != Some(true) {
            continue;
        }
        if !sink.send(event).await {
            return Ok(());
        }
    }
}

/// Every terminal event the viewer is allowed to see, until the socket closes.
///
/// A `started` or `restarted` event carries the session's `history`, so this
/// gate is what keeps another viewer's terminal output off the socket.
pub(crate) async fn watch_terminal_events(
    sink: subscription::StreamSink,
    mut events: tokio::sync::broadcast::Receiver<Value>,
    tasks: Arc<dyn moatless_ports::task::TaskReader>,
    viewer: ScopeContext,
) -> StreamResult {
    let mut readable = ReadableTasks::new(tasks, viewer.clone());
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                warn!(missed, user_id = %viewer.user_id, "A terminal event subscriber lagged");
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
        };
        let Some(task_id) = event.get("threadId").and_then(Value::as_str) else {
            continue;
        };
        if readable.allows(task_id).await != Some(true) {
            continue;
        }
        if !sink.send(event).await {
            return Ok(());
        }
    }
}

/// Every terminal summary the viewer is allowed to see: the hub's current set
/// first, then each change until the socket closes.
///
/// The relay puts every announced session in every replica's hub, and a summary
/// names a Task's `cwd`, `worktreePath` and `label`, so the seed needs the same
/// check as the events after it.
pub(crate) async fn watch_terminal_metadata(
    sink: subscription::StreamSink,
    hub: crate::terminal::TerminalHub,
    tasks: Arc<dyn moatless_ports::task::TaskReader>,
    viewer: ScopeContext,
) -> StreamResult {
    let mut metadata = hub.subscribe_metadata();
    let mut readable = ReadableTasks::new(tasks, viewer.clone());
    // Subscribed before the snapshot is taken, so a session starting between
    // the two arrives as an event rather than falling in the gap.
    let seed = hub.metadata_snapshot().await;
    let mut allowed = Vec::new();
    for summary in seed["terminals"].as_array().into_iter().flatten() {
        let Some(task_id) = summary.get("threadId").and_then(Value::as_str) else {
            continue;
        };
        if readable.allows(task_id).await == Some(true) {
            allowed.push(summary.clone());
        }
    }
    if !sink
        .send(serde_json::json!({ "type": "snapshot", "terminals": allowed }))
        .await
    {
        return Ok(());
    }
    loop {
        let event = match metadata.recv().await {
            Ok(event) => event,
            // Lagged, as above: the client reconciles from the next event it
            // does see rather than from a re-opened stream.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                warn!(missed, user_id = %viewer.user_id, "A terminal metadata subscriber lagged");
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
        };
        // `remove` names the thread at the top level, `upsert` inside the
        // summary it carries.
        let task_id = event
            .get("threadId")
            .or_else(|| event.pointer("/terminal/threadId"))
            .and_then(Value::as_str);
        let Some(task_id) = task_id else {
            continue;
        };
        if readable.allows(task_id).await != Some(true) {
            continue;
        }
        if !sink.send(event).await {
            return Ok(());
        }
    }
}
