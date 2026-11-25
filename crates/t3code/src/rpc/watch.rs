use crate::change_watch::ThreadSignal;
use crate::change_watch::next_change;
use crate::config::AgentModelDefaults;
use crate::dao::ConversationDao;
use crate::feed;
use crate::poll_gate::first_tick;
use crate::projection::Compactions;
use crate::reads::T3Reads;
use crate::review;
use crate::rpc::metrics::THREAD_SUBSCRIPTIONS;
use crate::snapshots::SnapshotShape;
use crate::snapshots::ThreadDetailV2;
use crate::snapshots::thread_detail_v2_under;
use crate::state::Deployment;
use crate::subscription;
use crate::subscription::StreamFailure;
use crate::subscription::StreamResult;
use crate::subscriptions::SHELL_POLL_INTERVAL;
use crate::subscriptions::THREAD_POLL_INTERVAL;
use crate::subscriptions::VCS_STATUS_POLL_INTERVAL;
use feed::ShellFeed;
use moatless_core::change_feed::conversation::ConversationHead;
use moatless_core::change_feed::listing::ListingHead;
use moatless_types::access_control::ScopeContext;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
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
    let mut pause = first_tick(VCS_STATUS_POLL_INTERVAL);
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
        tokio::time::sleep(pause).await;
        pause = VCS_STATUS_POLL_INTERVAL;
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
        // Every shell subscription wakes on the same move, so the gate is what
        // spreads their reads.
        let polled = feed.poll_at(&head, &reads, &model_defaults, &viewer);
        let values = match reads.poll_gate().run(polled).await {
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
    /// Applied to every snapshot after the first, none of which is bounded.
    pub(crate) compactions: Compactions,
    pub(crate) signals: watch::Receiver<Option<ThreadSignal>>,
}

/// The `snapshot` stream item a V2 thread subscription sends, shaped as
/// [`ThreadDetailV2::take_shaped`] describes.
pub(crate) fn thread_snapshot_item(detail: &mut ThreadDetailV2, shape: SnapshotShape) -> Value {
    detail.take_shaped(json!({ "kind": "snapshot" }), shape)
}

/// Answers every move of the conversation's head with the whole projection
/// rather than the events between.
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
        );
        let mut detail = match watched.reads.poll_gate().run(detail).await {
            Ok(Some(detail)) => detail,
            Ok(None) => return Err(StreamFailure::message("Conversation not found")),
            Err(error) => {
                // The head is not advanced, so the next pass rebuilds the
                // same snapshot.
                warn!(%error, user_id = %watched.viewer.user_id, "Failed to assemble the T3 UI conversation");
                tokio::time::sleep(THREAD_POLL_INTERVAL).await;
                continue;
            }
        };
        watched.head = detail.head;
        let shape = SnapshotShape {
            bounded: false,
            compactions: watched.compactions,
        };
        if !sink.send(thread_snapshot_item(&mut detail, shape)).await {
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
    owned_only: bool,
    answered: HashMap<String, bool>,
}

impl ReadableTasks {
    fn new(tasks: Arc<dyn moatless_ports::task::TaskReader>, viewer: ScopeContext) -> Self {
        Self {
            tasks,
            viewer,
            owned_only: false,
            answered: HashMap::new(),
        }
    }

    /// Narrowed to the Tasks the viewer owns, for a stream whose events only
    /// an owner can act on.
    fn owned(tasks: Arc<dyn moatless_ports::task::TaskReader>, viewer: ScopeContext) -> Self {
        Self {
            owned_only: true,
            ..Self::new(tasks, viewer)
        }
    }

    /// `None` when the database could not say, which the caller must treat as
    /// "do not send" rather than as a refusal to cache.
    async fn allows(&mut self, task_id: &str) -> Option<bool> {
        if let Some(answered) = self.answered.get(task_id) {
            return Some(*answered);
        }
        let answer = if self.owned_only {
            self.tasks
                .owned(task_id, &self.viewer)
                .await
                .map(|task| task.is_some())
        } else {
            self.tasks.may_access(task_id, &self.viewer).await
        };
        match answer {
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
/// The shortest gap between two refreshes one client is told of. Each sends
/// it to re-read every pull request it shows, against the viewer's own GitHub
/// quota, so a burst of webhooks is answered once.
const PULL_REQUEST_REFRESH_GAP: Duration = Duration::from_secs(5);

/// `pullRequests.subscribeRefreshes`: the host-change count, each time it
/// moves past the one this subscription opened at.
///
/// Not narrowed to what the viewer can read: the count names no pull request,
/// and a client re-reads only its own.
pub(crate) async fn watch_pull_request_refreshes(
    sink: subscription::StreamSink,
    mut changes: watch::Receiver<Option<i64>>,
) -> StreamResult {
    // The first read is where this client starts, not a change.
    let Some(mut seen) = next_change(&mut changes, None, |_| true).await else {
        return Ok(());
    };
    loop {
        let Some(count) = next_change(&mut changes, Some(&seen), |count| *count > 0).await else {
            return Ok(());
        };
        seen = count;
        if !sink.send(json!(count)).await {
            return Ok(());
        }
        tokio::time::sleep(PULL_REQUEST_REFRESH_GAP).await;
    }
}

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

/// Every summary of a Task the viewer owns, seed then changes. Owned, not read:
/// an `announced` upsert makes the client attach, which is owner-only, and the
/// seed is checked too because every hub holds every Task's summaries.
pub(crate) async fn watch_terminal_metadata(
    sink: subscription::StreamSink,
    hub: crate::terminal::TerminalHub,
    tasks: Arc<dyn moatless_ports::task::TaskReader>,
    viewer: ScopeContext,
) -> StreamResult {
    let mut metadata = hub.subscribe_metadata();
    let mut readable = ReadableTasks::owned(tasks, viewer.clone());
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
