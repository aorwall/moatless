use async_trait::async_trait;
use moatless_types::store::StoreError;

/// Where a turn came from, as the Task's transcript records it.
///
/// `event_id` is what dedup keys on, so a re-delivery or an echo of an outbound
/// send never becomes a second turn.
#[derive(Debug, Clone, Default)]
pub struct TurnOrigin {
    pub event_id: Option<String>,
    pub adapter: Option<String>,
    pub subject_key: Option<String>,
    pub source_name: Option<String>,
    /// The handle the external system knows the writer by, recorded verbatim
    /// and never resolved to a Moatless identity.
    pub author: Option<String>,
    /// The writer's own id in the external system — a Slack `U0000000001`. What
    /// the sender label resolves through, since the Moatless user id on the
    /// send is the subscription's owner when the writer is unmapped.
    pub sender_id: Option<String>,
}

/// A turn arriving from outside the product, addressed to a Task that exists.
///
/// One struct rather than a parameter list: the flow it feeds already carried
/// `#[allow(clippy::too_many_arguments)]`.
#[derive(Debug, Clone)]
pub struct InboundTurn {
    pub task_id: String,
    pub text: String,
    pub file_ids: Vec<String>,
    /// Continue the agent's current session rather than starting fresh.
    pub resume: bool,
    pub origin: TurnOrigin,
    /// The identity the turn is attributed to, when the Adapter resolved one.
    pub sent_by: Option<String>,
    /// Skills the sandbox activates for this turn.
    pub skills: Vec<String>,
}

/// Accepting a turn onto an existing Task.
///
/// Distinct from [`super::TranscriptAppend`], which writes one row and runs no
/// flow; this deduplicates, persists, and hands the turn to the agent.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait MessageAcceptor: Send + Sync {
    /// A turn a provider already deduplicated away, and a Task that is closed,
    /// are both `Ok(())` with nothing written — there is no queue to join.
    async fn accept(&self, turn: InboundTurn) -> Result<(), StoreError>;
}
