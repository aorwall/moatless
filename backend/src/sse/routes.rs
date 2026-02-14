//! SSE endpoint handler.
//!
//! Serves `GET /api/v1/events/stream` as an NDJSON stream, fanning out
//! events from the `tokio::sync::broadcast` channel to each connected client.

use async_stream::stream;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use futures::stream::StreamExt;
use serde::Deserialize;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

/// Shared state for the SSE route.
#[derive(Clone)]
pub struct SseState {
    pub broadcast: broadcast::Sender<String>,
    pub cancel: CancellationToken,
}

/// Query parameters for the SSE endpoint.
#[derive(Deserialize)]
pub struct SseQuery {
    /// Comma-separated event type filter (e.g., `?types=agent,environment`).
    pub types: Option<String>,
}

/// Build the SSE router.
pub fn sse_router(state: SseState) -> Router {
    Router::new()
        .route("/api/v1/events/stream", get(sse_handler))
        .with_state(state)
}

/// SSE endpoint handler.
///
/// Returns an NDJSON stream with:
/// - Event lines: raw JSON from Redis + `\n`
/// - Ping every 30s: `{"type":"ping","timestamp":"..."}\n`
async fn sse_handler(
    State(state): State<SseState>,
    Query(query): Query<SseQuery>,
) -> Response {
    let rx = state.broadcast.subscribe();
    let cancel = state.cancel.clone();

    // Parse type filters
    let type_filters: Vec<String> = query
        .types
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let ndjson_stream = stream! {
        let mut rx = rx;
        let ping_interval = std::time::Duration::from_secs(30);
        let mut ping_timer = tokio::time::interval(ping_interval);
        // Skip the first immediate tick
        ping_timer.tick().await;

        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    break;
                }
                result = rx.recv() => {
                    match result {
                        Ok(payload) => {
                            // Apply type filter if specified
                            if !type_filters.is_empty() {
                                let matches = type_filters.iter().any(|t| {
                                    // Substring match on the raw JSON for "type":"<value>"
                                    let pattern = format!("\"type\":\"{t}\"");
                                    payload.contains(&pattern)
                                });
                                if !matches {
                                    continue;
                                }
                            }

                            let line = format!("{payload}\n");
                            yield Ok::<_, std::convert::Infallible>(line);
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(skipped = n, "SSE client lagged, skipping events");
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            debug!("SSE broadcast channel closed");
                            break;
                        }
                    }
                }
                _ = ping_timer.tick() => {
                    let now = chrono::Utc::now().to_rfc3339_opts(
                        chrono::SecondsFormat::Millis,
                        true,
                    );
                    let ping = format!("{{\"type\":\"ping\",\"timestamp\":\"{now}\"}}\n");
                    yield Ok::<_, std::convert::Infallible>(ping);
                }
            }
        }
    };

    // Convert to axum body
    let body = Body::from_stream(ndjson_stream.map(|r| r.map(std::string::String::into_bytes)));

    Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("X-Accel-Buffering", "no")
        .body(body)
        .unwrap()
}
