use crate::config::AgentModelDefaults;
use crate::dao::ListingError;
use crate::feed;
use crate::reads::T3Reads;
use crate::snapshots::shell_snapshot;
use crate::workspace_listing::WorkspaceListing;
use feed::ShellFeed;
use moatless_core::change_feed::listing;
use moatless_core::change_feed::listing::EnvironmentIdentity;
use moatless_types::access_control::ScopeContext;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;
use tracing::debug;
use tracing::info;

/// The contract's `OrchestrationV2SubscribeShellInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationV2SubscribeShellInput {
    #[serde(default, deserialize_with = "crate::snapshots::after_sequence")]
    pub(crate) after_sequence: Option<i64>,
    pub(crate) request_completion_marker: Option<bool>,
}

/// The contract's `OrchestrationV2SubscribeThreadInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationV2SubscribeThreadInput {
    pub(crate) thread_id: String,
    #[serde(default, deserialize_with = "crate::snapshots::after_sequence")]
    pub(crate) after_sequence: Option<i64>,
    pub(crate) request_completion_marker: Option<bool>,
    #[serde(default)]
    pub(crate) accept_bounded_snapshot: bool,
    #[serde(default)]
    pub(crate) accept_compact_turn_items: bool,
    #[serde(default)]
    pub(crate) accept_deduped_bodies: bool,
    #[serde(default)]
    pub(crate) accept_compact_ids: bool,
}

/// How often [`ChangePoller`](crate::change_watch::ChangePoller) reads the
/// listing head, so how often a shell subscription can learn of a change.
///
/// Also the coalescing window: everything recorded for one conversation in it
/// collapses into a single re-read of that row.
pub(crate) const SHELL_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How often [`ChangePoller`](crate::change_watch::ChangePoller) reads the
/// heads of the open conversations.
///
/// Longer than the shell's: the question is cheap (a fingerprint), but a hit
/// delivers the whole conversation.
pub(crate) const THREAD_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How often an open conversation asks its Sandbox about its branch state.
///
/// Each pass is a gRPC round trip running `git` in the pod, so this is far
/// longer than the two above; the panel that reads it most only needs it once.
pub(crate) const VCS_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(15);

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
) -> Result<ShellSubscription, ListingError> {
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
            let mut feed = ShellFeed::resuming_from(head.identity, cursor).grouping_by(grouping);
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
            let snapshot = shell_snapshot(reads, model_defaults, viewer).await?;
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
