use crate::config::AgentModelDefaults;
use crate::config::OrchestrationProtocol;
use crate::feed;
use crate::reads::T3Reads;
use crate::snapshots::shell_snapshot_in;
use crate::snapshots::thread_snapshot;
use crate::state::Deployment;
use crate::workspace_listing::WorkspaceListing;
use feed::ShellFeed;
use feed::ThreadFeed;
use moatless_core::change_feed::conversation::Resume;
use moatless_core::change_feed::conversation::resume;
use moatless_core::change_feed::listing;
use moatless_core::change_feed::listing::EnvironmentIdentity;
use moatless_types::access_control::ScopeContext;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;
use tracing::debug;
use tracing::info;

/// How often a shell subscription asks what has changed.
///
/// Also the coalescing window: everything recorded for one conversation in it
/// collapses into a single re-read of that row.
pub(crate) const SHELL_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How often an open conversation asks whether it has moved.
///
/// Longer than the shell's: the question is cheap (a fingerprint), but a hit
/// delivers the whole conversation.
pub(crate) const THREAD_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How often an open conversation asks its Sandbox about its branch state.
///
/// Each pass is a gRPC round trip running `git` in the pod, so this is far
/// longer than the two above; the panel that reads it most only needs it once.
pub(crate) const VCS_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(15);

/// Open a subscription to one conversation: either replay what a returning
/// client missed, or hand it the conversation built afresh.
///
/// `after` is the client's last known sequence; [`resume`] decides which path.
/// Replay emits `event` items to fold in; rebuild emits one `snapshot` that
/// replaces what it holds. `Ok(None)` when not visible to this viewer.
pub async fn thread_subscription(
    reads: &T3Reads,
    deployment: &Deployment<'_>,
    viewer: &ScopeContext,
    task_id: &str,
    after: Option<i64>,
    request_completion_marker: bool,
) -> Result<Option<(Vec<Value>, ThreadFeed)>, sqlx::Error> {
    let conversations = reads.conversations();
    let head = conversations.conversation_head(task_id).await?;
    // Qualified: the shell has a decision of the same shape over a different
    // log, and the two cursors are not interchangeable.
    let resume = match after {
        Some(after) => resume(head, after),
        None => Resume::Rebuild,
    };
    if let Resume::From(cursor) = resume {
        // No access check of its own: the first poll below makes one, silent
        // or not, and answers `None` for a conversation this viewer may not see.
        let mut feed = ThreadFeed::resuming_from(task_id.to_string(), cursor);
        let Some(values) = poll_thread(&mut feed, reads, deployment, viewer).await? else {
            return Ok(None);
        };
        debug!(
            user_id = %viewer.user_id,
            task_id,
            cursor,
            replayed = values.len(),
            "T3 UI thread subscription resumed from the client's position"
        );
        return Ok(Some((
            thread_initial_values(values, request_completion_marker),
            feed,
        )));
    }

    let Some(snapshot) = thread_snapshot(reads, deployment, viewer, task_id).await? else {
        return Ok(None);
    };
    if let Some(after) = after {
        info!(
            user_id = %viewer.user_id,
            task_id,
            after,
            head = snapshot.head.sequence(),
            "T3 UI thread subscription could not replay from the client's position; \
             sending the conversation afresh"
        );
    }
    let feed = ThreadFeed::at(task_id.to_string(), snapshot.head, snapshot.checkpointed);
    let values = vec![json!({
        "kind": "snapshot",
        "snapshot": snapshot.value
    })];
    Ok(Some((
        thread_initial_values(values, request_completion_marker),
        feed,
    )))
}

/// Poll `feed`, then answer with a fresh snapshot and restart `feed` from it
/// when the poll found more settled turns than it could send checkpoints for.
///
/// A failed snapshot leaves [`ThreadFeed::needs_snapshot`] set, so the next
/// poll retries it.
pub(crate) async fn poll_thread(
    feed: &mut ThreadFeed,
    reads: &T3Reads,
    deployment: &Deployment<'_>,
    viewer: &ScopeContext,
) -> Result<Option<Vec<Value>>, sqlx::Error> {
    let Some(mut values) = feed
        .poll(reads.conversations(), viewer, deployment.frontend_url)
        .await?
    else {
        return Ok(None);
    };
    if feed.needs_snapshot() {
        let task_id = feed.task_id().to_string();
        let Some(snapshot) = thread_snapshot(reads, deployment, viewer, &task_id).await? else {
            return Ok(None);
        };
        info!(
            task_id,
            "T3 UI thread subscription had more settled turns to send than one change holds; \
             sending the conversation afresh"
        );
        *feed = ThreadFeed::at(task_id, snapshot.head, snapshot.checkpointed);
        values.push(json!({ "kind": "snapshot", "snapshot": snapshot.value }));
    }
    Ok(Some(values))
}

pub(crate) fn thread_initial_values(
    mut values: Vec<Value>,
    request_completion_marker: bool,
) -> Vec<Value> {
    if request_completion_marker {
        values.push(json!({ "kind": "synchronized" }));
    }
    values
}

/// What a subscribing client is sent before live delivery starts, and the feed
/// that carries on from it.
pub struct ShellSubscription {
    /// The stream items for the first chunk, in the order the client applies
    /// them. Empty means the client was already up to date — and *only* that,
    /// which is the distinction this ticket exists to make.
    pub values: Vec<Value>,
    pub feed: ShellFeed,
}

/// Open a subscription: either replay what a returning client missed, or hand
/// it a listing built afresh.
///
/// `environment` mismatching the head is [`listing::resume`]'s third rebuild
/// reason, catching a position handed out to a different database.
pub async fn shell_subscription(
    reads: &T3Reads,
    model_defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    environment: &EnvironmentIdentity,
    after: Option<i64>,
    protocol: OrchestrationProtocol,
) -> Result<ShellSubscription, sqlx::Error> {
    let conversations = reads.conversations();
    let workspaces = reads.workspaces();
    let head = conversations.listing_head().await?;
    let resume = match after {
        Some(after) => listing::resume(environment, &head.identity, head.position, after),
        None => listing::Resume::Rebuild,
    };
    match resume {
        listing::Resume::From(cursor) => {
            // Positioned, holding nothing it may take back, and grouping by the
            // projects this viewer can read. See `resume_grouping`.
            let grouping = resume_grouping(workspaces, viewer).await?;
            let mut feed = ShellFeed::resuming_from(head.identity, cursor)
                .grouping_by(grouping)
                .speaking(protocol);
            let values = feed.poll(reads, model_defaults, viewer).await?;
            debug!(
                user_id = %viewer.user_id,
                cursor,
                replayed = values.len(),
                "T3 UI shell subscription resumed from the client's position"
            );
            Ok(ShellSubscription { values, feed })
        }
        listing::Resume::Rebuild => {
            let snapshot = shell_snapshot_in(reads, model_defaults, viewer, protocol).await?;
            if let Some(after) = after {
                info!(
                    user_id = %viewer.user_id,
                    after,
                    head = snapshot.sequence,
                    retained = listing::RETAINED_CHANGES,
                    replaced_database = snapshot.identity != *environment,
                    "T3 UI shell subscription could not replay from the client's \
                     position; sending a freshly built listing"
                );
            }
            // What the client will be holding the moment it applies this, and
            // therefore the only ids its feed may ever take back. Read off the
            // projected listing rather than the rows behind it — see
            // [`ShellSnapshot::thread_ids`].
            let (threads, projects) = (snapshot.thread_ids(), snapshot.project_ids());
            let feed = ShellFeed::resuming_from(snapshot.identity, snapshot.sequence)
                .holding(threads, projects)
                .speaking(protocol);
            Ok(ShellSubscription {
                feed,
                values: vec![json!({
                    "kind": "snapshot",
                    "snapshot": snapshot.value
                })],
            })
        }
    }
}

/// The projects a resuming client can be assumed to have a group for.
///
/// **Not the set of ids it may be sent a removal for** — see [`ShellFeed`]. A
/// project held only because a listed conversation references it is left out.
pub(crate) async fn resume_grouping(
    workspaces: &dyn WorkspaceListing,
    viewer: &ScopeContext,
) -> Result<Vec<String>, sqlx::Error> {
    Ok(workspaces
        .list_readable_including(viewer, &[])
        .await?
        .into_iter()
        .map(|listed| listed.workspace.id)
        .collect())
}
