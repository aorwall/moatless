//! SSE routing table: the live firehose (`GET /api/v1/events/stream`), served
//! by [`super::handlers`].

use axum::Router;
use axum::routing::get;

use super::handlers::sse_handler;

pub use super::state::SseState;

/// Build the live SSE firehose router (state-independent from the app).
pub fn sse_router(state: SseState) -> Router {
    Router::new()
        .route("/api/v1/events/stream", get(sse_handler))
        .with_state(state)
}
