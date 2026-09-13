use moatless_ports::task::TaskReader;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

/// Shared state for the live SSE firehose (`GET /api/v1/events/stream`).
#[derive(Clone)]
pub struct SseState {
    pub broadcast: broadcast::Sender<String>,
    pub cancel: CancellationToken,
    /// Task access resolver, used to filter the firehose per tenant so a
    /// non-admin client only receives events for tasks it can read.
    ///
    /// `None` when the DB is unconfigured, disabling the firehose for scoped
    /// users (see `sse_handler`).
    pub tasks: Option<Arc<dyn TaskReader>>,
}
