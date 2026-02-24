//! SSE endpoint handler.
//!
//! Serves `GET /api/v1/events/stream` as an NDJSON stream, fanning out
//! events from the `tokio::sync::broadcast` channel to each connected client.

use async_stream::stream;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use futures::stream::StreamExt;
use serde::Deserialize;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::sse::consumer::extract_field;

/// Shared state for the SSE route.
#[derive(Clone)]
pub struct SseState {
    pub broadcast: broadcast::Sender<String>,
    pub cancel: CancellationToken,
    /// Optional Redis connection for historical event queries.
    pub redis: Option<redis::aio::ConnectionManager>,
}

/// Query parameters for the SSE endpoint.
#[derive(Deserialize)]
pub struct SseQuery {
    /// Comma-separated event type filter (e.g., `?types=agent,environment`).
    pub types: Option<String>,
}

/// Query parameters for the historical task events endpoint.
#[derive(Deserialize)]
pub struct TaskEventsQuery {
    /// Comma-separated event type filter (e.g., `?types=sandbox,server`).
    pub types: Option<String>,
    /// Maximum number of events to return (default 500, max 2000).
    pub limit: Option<usize>,
}

/// Build the SSE router.
pub fn sse_router(state: SseState) -> Router {
    Router::new()
        .route("/api/v1/events/stream", get(sse_handler))
        .route("/api/v1/tasks/{task_id}/events", get(task_events_handler))
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

    #[allow(clippy::expect_used)] // Response::builder() only fails with invalid header names (ours are static)
    Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("X-Accel-Buffering", "no")
        .body(body)
        .expect("valid SSE response with static header names")
}

/// Historical task events handler.
///
/// Returns stored events from Redis Streams for a given task, allowing the
/// frontend to hydrate the Events tab on page reload.
async fn task_events_handler(
    State(state): State<SseState>,
    Path(task_id): Path<String>,
    Query(query): Query<TaskEventsQuery>,
) -> Json<Vec<serde_json::Value>> {
    let Some(mut conn) = state.redis.clone() else {
        return Json(vec![]);
    };

    let limit = query.limit.unwrap_or(500).min(2000);
    let stream_key = format!("task-events:{task_id}");

    // Parse type filters
    let type_filters: Vec<String> = query
        .types
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    // XRANGE task-events:{task_id} - + COUNT {limit}
    let result: redis::Value = match redis::cmd("XRANGE")
        .arg(&stream_key)
        .arg("-")
        .arg("+")
        .arg("COUNT")
        .arg(limit)
        .query_async(&mut conn)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to read task events from Redis");
            return Json(vec![]);
        }
    };

    let entries = match &result {
        redis::Value::Array(arr) => arr,
        _ => return Json(vec![]),
    };

    let mut events = Vec::with_capacity(entries.len());

    for entry in entries {
        let fields = match entry {
            redis::Value::Array(pair) if pair.len() == 2 => match &pair[1] {
                redis::Value::Array(f) => f,
                _ => continue,
            },
            _ => continue,
        };

        let Some(payload_str) = extract_field(fields, "payload") else {
            continue;
        };

        // Apply type filter if specified
        if !type_filters.is_empty() {
            let matches = type_filters.iter().any(|t| {
                let pattern = format!("\"type\":\"{t}\"");
                payload_str.contains(&pattern)
            });
            if !matches {
                continue;
            }
        }

        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload_str) {
            events.push(value);
        }
    }

    Json(events)
}
