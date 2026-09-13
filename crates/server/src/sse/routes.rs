//! SSE routing tables.
//!
//! Assembles the live firehose (`GET /api/v1/events/stream`) and the historical
//! task-events (`GET /api/v1/tasks/{task_id}/events`) routers. The handlers
//! they mount live in [`super::handlers`].

use axum::Router;
use axum::routing::get;

use crate::state::AppState;

use super::handlers::{sse_handler, task_events_handler};

pub use super::state::SseState;

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
