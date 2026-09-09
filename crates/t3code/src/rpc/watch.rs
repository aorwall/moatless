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

/// Report one conversation's branch state until its socket goes away.
///
/// Polled rather than pushed: the state lives in a Sandbox's checkout,
/// which changes when an agent writes files and tells nobody. The interval
/// is long because of what reads it — the panel picks its opening scope
/// from `hasWorkingTreeChanges` and then reads patches through
/// `review.getDiffPreview`, which the client refetches on its own.
///
/// A pass that cannot reach the Sandbox says nothing and waits for the next
/// one. Ending the stream would present a stopped or still-starting sandbox
/// — the ordinary state of most conversations — as a permanent answer the
/// client would then hold until it reconnects.
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

/// Watch the listing feed for one subscription until its socket goes away.
///
/// Each pass delivers at most one chunk, however many changes it read: a
/// burst on one conversation is coalesced into one re-read of that row, and
/// the poll interval is the window that does the coalescing.
///
/// Chunks are not gated on the client's `Ack`, which the reference server
/// uses for backpressure. The client acks a chunk as soon as it has queued
/// its values (`RpcClient`'s `Chunk` case offers to the queue, then acks),
/// so it does not require the server to wait — and the poll interval already
/// bounds this stream to four chunks a second regardless of what changed.
///
/// The feed arrives built by [`shell_subscription`] — positioned at the
/// snapshot's place or at wherever the catch-up read reached, and holding
/// whatever that left the client holding. Both are facts about how the
/// subscription opened and neither is this loop's to decide, which is also
/// why nothing between the first chunk and the first live poll can fall
/// through the join: it is the same feed either way.
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

/// Push one conversation's events for as long as the subscription lives.
///
/// The shape mirrors [`watch_shell`] over a change log of its own, and
/// runs at a quarter of the rate: a conversation records a position per
/// Message block where the shell records one per Task, so a poll here reads
/// a burst rather than a row.
///
/// A poll has a third answer the shell's cannot produce: `None`, meaning the
/// conversation is no longer readable — deleted, unfollowed, or its project
/// ungrouped. The subscription cannot recover from any of those by polling
/// again, so it ends rather than idling silently on a screen that would keep
/// rendering the last snapshot as current.
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

/// Send every preview event the viewer is allowed to see.
///
/// The subscription is environment-wide — the contract's payload is empty
/// and the client filters by thread itself — so this is the one place a
/// tab opened on a Task the viewer cannot read would otherwise reach them.
/// Each Task is authorized once per subscription and the answer is
/// remembered: tab events are rare, and a scoped read per event would put
/// a database round trip on a fan-out path.
pub(crate) async fn watch_preview_events(
    sink: subscription::StreamSink,
    preview: crate::preview::PreviewState,
    viewer: ScopeContext,
) -> StreamResult {
    let mut events = preview.tabs.subscribe();
    let mut readable: HashMap<String, bool> = HashMap::new();
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
        let allowed = if let Some(allowed) = readable.get(task_id) {
            *allowed
        } else {
            let allowed = match preview.tasks.find_by_id(task_id, &viewer).await {
                Ok(task) => task.is_some(),
                Err(error) => {
                    warn!(%error, task_id, "Could not authorize a preview event");
                    continue;
                }
            };
            readable.insert(task_id.to_string(), allowed);
            allowed
        };
        if !allowed {
            continue;
        }
        if !sink.send(event).await {
            return Ok(());
        }
    }
}
