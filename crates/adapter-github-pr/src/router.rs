//! The GitHub PR inbound-routing service.
//!
//! Everything a GitHub PR webhook does after admission — routing a message to
//! its bound tasks, spawning a review task, auto-closing on PR close, and the
//! post-merge / merge-conflict notices — reads a wide set of task, routing,
//! message, and binding ports that are NOT adapter infrastructure and so do not
//! belong on [`GithubPrAdapter`]. They also do not belong threaded one by one
//! through the delivery closures. This is the constructed-once provider-band
//! service that holds them: built at compose from the composition root's ports,
//! captured by [`super::delivery::register`], and driving the four delivery
//! handlers as methods — [`route_event`](Self::route_event) /
//! [`close_subject`](Self::close_subject) in [`super::routing`],
//! [`run_post_merge`](Self::run_post_merge) in [`super::post_merge`], and
//! [`run_merge_conflict`](Self::run_merge_conflict) in [`super::merge_conflict`].
//!
//! Every port it holds sits below the adapter band, so naming them here is a set
//! of downward edges, not the upward reach into `AppState` this replaced.

use std::sync::Arc;

use moatless_core::db::DbPool;

use moatless_loops::adapters::dao::AdapterConnectionDao;
use moatless_loops::bindings::recently_sent::RecentlySent;
use moatless_loops::bindings::runtime::RoutingService;
use moatless_loops::loops::dao::LoopDao;
use moatless_tasks::message::dao::MessageDao;
use moatless_tasks::message::send::MessageSendService;
use moatless_tasks::task::binding_dao::TaskBindingDao;
use moatless_tasks::task::closer::TaskCloser;
use moatless_tasks::task::creation::TaskCreationService;
use moatless_tasks::task::dao::TaskDao;
use moatless_tasks::task::relations::TaskRelationDao;

use super::adapter::GithubPrAdapter;

/// Constructed-once inbound-routing service for the GitHub PR adapter. Holds the
/// downward ports the four delivery handlers read; see the module doc.
pub struct GithubPrInboundRouter {
    /// Per-provider connection records — resolves a delivery to its connection.
    pub(super) adapter_connections: Arc<dyn AdapterConnectionDao>,
    /// Subscription/trigger lookup — resolves a delivery to its trigger.
    pub(super) loops: Arc<LoopDao>,
    /// Task↔subject bindings — selection, upsert, sibling guard, cursor bumps.
    pub(super) task_bindings: Arc<dyn TaskBindingDao>,
    /// Generic fan-out + external-id aliasing.
    pub(super) routing: Arc<RoutingService>,
    /// External-event-id → task lookup for author attachment.
    pub(super) messages: Arc<dyn MessageDao>,
    /// Records the related-to relation on review-task spawn.
    pub(super) task_relations: Arc<dyn TaskRelationDao>,
    /// Push-echo dedup — which task claimed a `synchronize` sha.
    pub(super) recently_sent: Arc<RecentlySent>,
    /// Task rows — the close pre-check (already-closed / not-found skip).
    pub(super) tasks: Arc<dyn TaskDao>,
    /// Sender-user resolution for the synthetic close-notice message, and the
    /// durable-work credential the merge / post-merge handlers authenticate with.
    pub(super) db: DbPool,
    /// The shared task-creation spine (review-task spawn).
    pub(super) task_creation: Arc<TaskCreationService>,
    /// Delivers the PR-close notice into a bound task's chat.
    pub(super) message_send: Arc<MessageSendService>,
    /// Terminal close (update Closed → remove sandbox → deactivate → publish),
    /// which [`close_subject`](Self::close_subject) delegates to with an enriched
    /// close event.
    pub(super) task_closer: Arc<TaskCloser>,
    /// The concrete adapter — `Some` when GitHub PR is configured. Source of the
    /// self-resolved reply instructions and the merge / post-merge credential.
    pub(super) adapter: Option<Arc<GithubPrAdapter>>,
}

impl GithubPrInboundRouter {
    /// Wire the router from the composition root's ports.
    #[expect(
        clippy::too_many_arguments,
        reason = "the inbound-routing substrate is genuinely wide — thirteen downward ports the four delivery handlers read. Bundling them into a sub-struct would only shuffle the same fields behind a one-use type; the width is paid once here at construction rather than threaded through register, the four delivery closures, and every handler, which is the whole point of a constructed-once service."
    )]
    pub fn new(
        adapter_connections: Arc<dyn AdapterConnectionDao>,
        loops: Arc<LoopDao>,
        task_bindings: Arc<dyn TaskBindingDao>,
        routing: Arc<RoutingService>,
        messages: Arc<dyn MessageDao>,
        task_relations: Arc<dyn TaskRelationDao>,
        recently_sent: Arc<RecentlySent>,
        tasks: Arc<dyn TaskDao>,
        db: DbPool,
        task_creation: Arc<TaskCreationService>,
        message_send: Arc<MessageSendService>,
        task_closer: Arc<TaskCloser>,
        adapter: Option<Arc<GithubPrAdapter>>,
    ) -> Self {
        Self {
            adapter_connections,
            loops,
            task_bindings,
            routing,
            messages,
            task_relations,
            recently_sent,
            tasks,
            db,
            task_creation,
            message_send,
            task_closer,
            adapter,
        }
    }
}
