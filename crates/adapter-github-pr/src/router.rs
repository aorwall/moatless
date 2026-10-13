//! The GitHub PR inbound-routing service.
//!
//! Everything a GitHub PR webhook does after admission reads a wide set of
//! task, routing, message, and binding ports — not adapter infrastructure,
//! so they don't belong on [`GithubPrAdapter`] or threaded through delivery
//! closures.
//!
//! Built once at compose from the composition root's ports and captured by
//! [`super::delivery::register`]; every port here is a downward edge, not
//! the upward reach into `AppState`.

use std::sync::Arc;

use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::loops::LoopReader;
use moatless_loops::bindings::operations::Bindings;
use moatless_loops::bindings::runtime::RoutingService;
use moatless_ports::identity::ProviderIdentityLink;
use moatless_ports::task::MessageAcceptor;
use moatless_ports::task::RelationRecorder;
use moatless_ports::task::TaskFinisher;
use moatless_ports::task::TaskOpener;
use moatless_ports::task::{TaskReader, TranscriptAppend};

use super::adapter::GithubPrAdapter;

/// Constructed-once inbound-routing service for the GitHub PR adapter. Holds
/// the downward ports the delivery handlers read; see the module doc.
pub struct GithubPrInboundRouter {
    /// Per-provider connection records — resolves a delivery to its connection.
    pub(super) adapter_connections: Arc<dyn ConnectionLookup>,
    /// Subscription/trigger lookup — resolves a delivery to its trigger.
    pub(super) loops: Arc<dyn LoopReader>,
    /// Task↔subject bindings — selection, upsert, sibling guard, cursor bumps.
    pub(super) task_bindings: Arc<Bindings>,
    /// Resolves an inbound GitHub handle to the system user it belongs to.
    pub(super) users: Arc<dyn ProviderIdentityLink>,
    /// Generic fan-out + external-id aliasing.
    pub(super) routing: Arc<RoutingService>,
    /// External-event-id → task lookup for author attachment.
    pub(super) messages: Arc<dyn TranscriptAppend>,
    /// Records the related-to relation on review-task spawn.
    pub(super) task_relations: Arc<dyn RelationRecorder>,
    /// Task rows — the close pre-check (already-closed / not-found skip).
    pub(super) tasks: Arc<dyn TaskReader>,
    /// The shared task-creation spine (review-task spawn).
    pub(super) task_creation: Arc<dyn TaskOpener>,
    /// Delivers the PR-close notice into a bound task's chat.
    pub(super) message_send: Arc<dyn MessageAcceptor>,
    /// Terminal close (update Closed → remove sandbox → deactivate → publish),
    /// which [`close_subject`](Self::close_subject) delegates to with an
    /// enriched close event.
    pub(super) task_closer: Arc<dyn TaskFinisher>,
    /// The concrete adapter — `Some` when GitHub PR is configured. Source of
    /// the self-resolved reply instructions and the merge / post-merge
    /// credential.
    pub(super) adapter: Option<Arc<GithubPrAdapter>>,
}

impl GithubPrInboundRouter {
    /// Wire the router from the composition root's ports.
    #[expect(
        clippy::too_many_arguments,
        reason = "the inbound-routing substrate is genuinely wide — twelve downward ports the delivery handlers read. Bundling them into a sub-struct would only shuffle the same fields behind a one-use type; the width is paid once here at construction rather than threaded through register, every delivery closure, and every handler, which is the whole point of a constructed-once service."
    )]
    pub fn new(
        adapter_connections: Arc<dyn ConnectionLookup>,
        loops: Arc<dyn LoopReader>,
        task_bindings: Arc<Bindings>,
        users: Arc<dyn ProviderIdentityLink>,
        routing: Arc<RoutingService>,
        messages: Arc<dyn TranscriptAppend>,
        task_relations: Arc<dyn RelationRecorder>,
        tasks: Arc<dyn TaskReader>,
        task_creation: Arc<dyn TaskOpener>,
        message_send: Arc<dyn MessageAcceptor>,
        task_closer: Arc<dyn TaskFinisher>,
        adapter: Option<Arc<GithubPrAdapter>>,
    ) -> Self {
        Self {
            adapter_connections,
            loops,
            task_bindings,
            users,
            routing,
            messages,
            task_relations,
            tasks,
            task_creation,
            message_send,
            task_closer,
            adapter,
        }
    }
}
