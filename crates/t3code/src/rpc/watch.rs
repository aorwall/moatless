use crate::dao::ConversationDao;
use crate::feed;
use crate::reads::T3Reads;
use crate::review;
use crate::subscription;
use crate::subscription::StreamFailure;
use crate::subscription::StreamResult;
use crate::subscriptions::SHELL_POLL_INTERVAL;
use crate::subscriptions::THREAD_POLL_INTERVAL;
use crate::subscriptions::VCS_STATUS_POLL_INTERVAL;
use feed::ShellFeed;
use feed::ThreadFeed;
use moatless_types::access_control::ScopeContext;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
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
/// chunk per pass regardless of how many rows changed.
///
/// Chunks are not gated on the client's `Ack` — it acks as soon as it has
/// queued the chunk — so the poll interval alone bounds this to four chunks a
/// second.
pub(crate) async fn watch_shell(
    sink: subscription::StreamSink,
    mut feed: ShellFeed,
    reads: T3Reads,
    model_defaults: Arc<crate::config::AgentModelDefaults>,
    viewer: ScopeContext,
) -> StreamResult {
    loop {
        tokio::time::sleep(SHELL_POLL_INTERVAL).await;
        let values = match feed.poll(&reads, &model_defaults, &viewer).await {
            Ok(values) => values,
            Err(error) => {
                // The subscription survives a failed poll. Ending it would
                // present a transient database error as a dead sidebar, and
                // the cursor has not moved, so the next poll covers the same
                // ground.
                //
                // That second clause is a fact about [`ShellFeed::poll`],
                // not about this loop: the cursor advances last, after the
                // reads that build the batch, so a failure in any of them
                // does not skip it silently. Do not move it back.
                warn!(
                    %error,
                    user_id = %viewer.user_id,
                    "Failed to read the T3 UI shell change feed"
                );
                continue;
            }
        };
        if values.is_empty() {
            continue;
        }
        // A chunk that cannot be queued is not the poll's problem, and the
        // cursor is deliberately left where the poll put it: the receiver
        // is held by the `ProtocolSession`, so a closed channel means the
        // socket loop has already returned and this subscription is over —
        // there is no next poll for a rewound cursor to serve. What the
        // client resumes from is the `afterSequence` of the snapshot it
        // re-fetches over HTTP before resubscribing, not anything this feed
        // remembers. Note that the converse also holds: a chunk queued
        // successfully may still fail on the wire in `send_frames`, so
        // treating the queue as delivery confirmation would be wrong in the
        // other direction too.
        if !sink.send_values(values).await {
            return Ok(());
        }
    }
}

/// Mirrors [`watch_shell`] over the conversation's own change log.
///
/// A poll can also answer `None` — the conversation is no longer readable
/// (deleted, unfollowed, ungrouped) — and the subscription ends rather than
/// idling on a stale snapshot.
pub(crate) async fn watch_thread(
    sink: subscription::StreamSink,
    mut feed: ThreadFeed,
    reads: T3Reads,
    frontend_url: Arc<str>,
    viewer: ScopeContext,
) -> StreamResult {
    loop {
        tokio::time::sleep(THREAD_POLL_INTERVAL).await;
        let polled = feed
            .poll(reads.conversations(), &viewer, &frontend_url)
            .await;
        let values = match polled {
            Ok(Some(values)) => values,
            Ok(None) => {
                return Err(StreamFailure::message("Conversation not found"));
            }
            Err(error) => {
                // As in `watch_shell`: a failed poll leaves the cursor where
                // it was, so the next one covers the same ground and the
                // subscription survives a transient database error.
                warn!(
                    %error,
                    user_id = %viewer.user_id,
                    "Failed to poll the T3 UI conversation"
                );
                continue;
            }
        };
        if values.is_empty() {
            continue;
        }
        if !sink.send_values(values).await {
            return Ok(());
        }
    }
}

/// Which Tasks one subscriber may read, answered once per Task per
/// subscription.
///
/// Both broadcasts below are environment-wide and the client filters by thread
/// for display only, so a stream that drops this check has no second gate
/// behind it.
struct ReadableTasks {
    tasks: Arc<dyn crate::task_dao::T3TaskDao>,
    viewer: ScopeContext,
    answered: HashMap<String, bool>,
}

impl ReadableTasks {
    fn new(tasks: Arc<dyn crate::task_dao::T3TaskDao>, viewer: ScopeContext) -> Self {
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
        match self.tasks.find_by_id(task_id, &self.viewer).await {
            Ok(task) => {
                let allowed = task.is_some();
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

/// Every terminal summary the viewer is allowed to see: the hub's current set
/// first, then each change until the socket closes.
///
/// The relay puts every announced session in every replica's hub, and a summary
/// names a Task's `cwd`, `worktreePath` and `label`, so the seed needs the same
/// check as the events after it.
pub(crate) async fn watch_terminal_metadata(
    sink: subscription::StreamSink,
    hub: crate::terminal::TerminalHub,
    tasks: Arc<dyn crate::task_dao::T3TaskDao>,
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
