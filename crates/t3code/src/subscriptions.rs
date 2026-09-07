use crate::config::AgentModelDefaults;
use crate::dao::ConversationDao;
use crate::feed;
use crate::listing;
use crate::listing::UiRpcEnvironmentIdentity;
use crate::listing::UiRpcResume;
use crate::snapshots::shell_snapshot;
use crate::snapshots::thread_snapshot;
use crate::state::Deployment;
use crate::workspace_dao::T3WorkspaceDao;
use feed::ShellFeed;
use feed::ThreadFeed;
use moatless_core::change_feed::conversation::Resume;
use moatless_core::change_feed::conversation::resume;
use moatless_types::access_control::ScopeContext;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;
use tracing::debug;
use tracing::info;

/// How often a shell subscription asks what has changed.
///
/// This interval is also the coalescing window: everything recorded for one
/// conversation inside it collapses into a single re-read of that row. Short
/// enough that a turn starting looks immediate, long enough that an agent
/// working flat out cannot turn the sidebar into a delivery loop.
pub(crate) const SHELL_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How often an open conversation asks whether it has moved.
///
/// Longer than the shell's because what it delivers is larger: the shell sends
/// one changed row, a conversation sends itself whole (see [`ThreadFeed`]). The
/// question asked each interval is cheap either way — a fingerprint, not a
/// transcript — so this is the coalescing window for the *answer*, and a second
/// is the shortest one at which an agent writing continuously does not turn a
/// transcript into a delivery loop.
pub(crate) const THREAD_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How often an open conversation asks its Sandbox about its branch state.
///
/// Each pass is a gRPC round trip to a pod that runs `git` for it, so this is
/// far longer than the two above: nothing on the screen is refreshed by it that
/// the client is not already refetching on its own, and the panel that reads it
/// most only needs it once, to pick which scope it opens on.
pub(crate) const VCS_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(15);

/// Open a subscription to one conversation: either replay what a returning
/// client missed, or hand it the conversation built afresh.
///
/// `after` is the sequence the client says it already holds — the
/// `snapshotSequence` of the bootstrap it fetched over HTTP moments earlier
/// (`state/threads.ts`), which is why the two have to be numbers in the same
/// namespace. [`resume`] is what decides between them, on the
/// same two grounds the shell's does: a sequence this conversation never
/// reached, and one further behind than the records kept.
///
/// The client separates the outcomes by decoding rather than by inference — a
/// rebuild is a `snapshot` item, which it applies by replacing what it holds,
/// and a replay is `event` items, which it folds in one at a time. So an
/// un-replayable gap is never silence.
///
/// # Why the catch-up is one ordinary poll
///
/// Replay reads through [`ThreadFeed::poll`], the same call the live
/// subscription makes every interval, so a resuming client is told exactly what
/// a client that never disconnected would have been told and there is no second
/// delivery path to drift from the first. The poll's own cap bounds the
/// catch-up, and the feed's cursor advances only over records it read, so
/// anything left over arrives on the next poll.
///
/// `Ok(None)` when the conversation is not visible to this viewer; the caller
/// answers the request rather than opening a stream that can only end.
pub async fn thread_subscription(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    deployment: &Deployment<'_>,
    viewer: &ScopeContext,
    task_id: &str,
    after: Option<i64>,
    request_completion_marker: bool,
) -> Result<Option<(Vec<Value>, ThreadFeed)>, sqlx::Error> {
    let head = conversations.conversation_head(task_id).await?;
    // Qualified: the shell has a decision of the same shape over a different
    // log, and the two cursors are not interchangeable.
    let resume = match after {
        Some(after) => resume(head, after),
        None => Resume::Rebuild,
    };
    if let Resume::From(cursor) = resume {
        // A replay reads nothing when nothing changed, and a poll that reads
        // nothing checks nothing. So the visibility rule is applied here as
        // well, or subscribing to a conversation this viewer may not see would
        // succeed for as long as it stayed still.
        if conversations
            .readable_by_id(viewer, task_id)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        let mut feed = ThreadFeed::resuming_from(task_id.to_string(), cursor);
        let Some(values) = feed
            .poll(conversations, viewer, deployment.frontend_url)
            .await?
        else {
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

    let Some(snapshot) =
        thread_snapshot(workspaces, conversations, deployment, viewer, task_id).await?
    else {
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
    let feed = ThreadFeed::at(task_id.to_string(), snapshot.head);
    let values = vec![json!({
        "kind": "snapshot",
        "snapshot": snapshot.value
    })];
    Ok(Some((
        thread_initial_values(values, request_completion_marker),
        feed,
    )))
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
/// `after` is the position the client says it already holds — T3's
/// `afterSequence`, absent for a client that holds nothing. `environment` is
/// the database this process started against; a database that no longer matches
/// it is [`listing::resume`]'s third reason to rebuild, and the only one
/// that catches a position handed out a second time to a different change.
///
/// # How the client tells a rebuild from silence
///
/// The two outcomes are different members of the contract's own
/// `OrchestrationShellStreamItem` union, so the client separates them by
/// decoding rather than by inference:
///
/// - **Replay** emits `thread-upserted` items, or none at all. The client folds
///   each into the snapshot it already holds (`shellReducer.ts`), keeping the
///   list it had. No items means nothing changed.
/// - **Rebuild** emits one `snapshot` item. The client *replaces* its snapshot
///   with it outright (`shell.ts:150-152`), which is what "freshly built
///   listing" means on that side.
///
/// So an un-replayable gap is never an empty answer — it is the same frame a
/// client with no snapshot at all receives (`apps/server/src/ws.ts:1297-1305`).
///
/// # The catch-up is one ordinary poll
///
/// Replay reads through [`ShellFeed::poll`], the same call the live
/// subscription makes every interval, so there is no second delivery path to
/// drift from the first. The poll's cap bounds the catch-up, and nothing can be
/// lost between the two: the feed's cursor advances only over records it read
/// and the same feed continues live, so anything left over arrives next poll.
pub async fn shell_subscription(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    model_defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    environment: &UiRpcEnvironmentIdentity,
    after: Option<i64>,
) -> Result<ShellSubscription, sqlx::Error> {
    let head = conversations.listing_head().await?;
    let resume = match after {
        Some(after) => listing::resume(environment, &head.identity, head.position, after),
        None => UiRpcResume::Rebuild,
    };
    match resume {
        UiRpcResume::From(cursor) => {
            // Positioned, holding nothing it may take back, and grouping by the
            // projects this viewer can read. See `resume_grouping`.
            let grouping = resume_grouping(workspaces, viewer).await?;
            let mut feed = ShellFeed::resuming_from(head.identity, cursor).grouping_by(grouping);
            let values = feed
                .poll(workspaces, conversations, model_defaults, viewer)
                .await?;
            debug!(
                user_id = %viewer.user_id,
                cursor,
                replayed = values.len(),
                "T3 UI shell subscription resumed from the client's position"
            );
            Ok(ShellSubscription { values, feed })
        }
        UiRpcResume::Rebuild => {
            let snapshot =
                shell_snapshot(workspaces, conversations, model_defaults, viewer).await?;
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
                .holding(threads, projects);
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
/// **Not the set of ids it may be sent a removal for**, which is a different
/// question with a different answer — see [`ShellFeed`]. This one only decides
/// whether a changed conversation can go out as a row or has to go out inside a
/// fresh listing, and getting it wrong costs a snapshot in one direction and a
/// row the client renders nowhere in the other.
///
/// A resuming client re-fetches the whole shell snapshot over HTTP and applies
/// it *before* it subscribes, sending that snapshot's own sequence as
/// `afterSequence` (`client-runtime/src/state/shell.ts:204-212`). So its
/// projects are the ones [`shell_snapshot`] returned, which is **every
/// workspace the viewer may read**, plus the ones a listed conversation
/// references.
///
/// This reads back the first half and not the second, and the asymmetry is the
/// point:
///
/// - A project the viewer may read now was in that snapshot unless it became
///   readable in between, and `ShellFeed::resynchronize` sends the whole
///   listing for a changed conversation entering it.
/// - A project held only because a conversation *references* it is left out.
///   That is the case where a conversation can arrive under a project the
///   client has never been sent, which `resynchronize` answers the same way.
///
/// Not covered: the viewer's own scope widening between their snapshot request
/// and their subscribe. The window is one round trip and the cost is one row
/// not rendered until the next rebuild, not a disclosure.
pub(crate) async fn resume_grouping(
    workspaces: &dyn T3WorkspaceDao,
    viewer: &ScopeContext,
) -> Result<Vec<String>, sqlx::Error> {
    Ok(workspaces
        .list_readable_including(viewer, &[])
        .await?
        .into_iter()
        .map(|listed| listed.workspace.id)
        .collect())
}
