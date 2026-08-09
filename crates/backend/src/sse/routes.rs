//! SSE routing tables.
//!
//! Assembles the live firehose (`GET /api/v1/events/stream`) and the historical
//! task-events (`GET /api/v1/tasks/{task_id}/events`) routers. The handlers they
//! mount live in [`super::handlers`].

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::state::AppState;
use crate::task::dao::TaskDao;

use super::handlers::{sse_handler, task_events_handler};

/// Shared state for the live SSE firehose (`GET /api/v1/events/stream`).
#[derive(Clone)]
pub struct SseState {
    pub broadcast: broadcast::Sender<String>,
    pub cancel: CancellationToken,
    /// Task access resolver, used to filter the firehose per tenant so a
    /// non-admin client only receives events for tasks it can read. `None`
    /// when the DB is unconfigured (no filtering possible → firehose disabled
    /// for scoped users; see `sse_handler`).
    pub tasks: Option<Arc<dyn TaskDao>>,
}

/// Build the live SSE firehose router (state-independent from the app).
pub fn sse_router(state: SseState) -> Router {
    Router::new()
        .route("/api/v1/events/stream", get(sse_handler))
        .with_state(state)
}

/// Build the historical task-events router. Kept on `AppState` (not `SseState`)
/// so the handler can authorize the caller against the task via `AuthUser`.
pub fn task_events_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/tasks/{task_id}/events", get(task_events_handler))
        .with_state(state)
}
