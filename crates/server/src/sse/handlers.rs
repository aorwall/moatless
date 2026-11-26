//! SSE endpoint handlers.
//!
//! Serves the live firehose (`GET /api/v1/events/stream`) as an NDJSON stream.
//! The router that mounts it is assembled in [`super::routes`].

use std::collections::HashMap;

use async_stream::stream;
use axum::body::Body;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::response::Response;
use futures::stream::StreamExt;
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::{debug, warn};

use moatless_types::access_control::ScopeContext;

use moatless_types::auth::AuthIdentity;
use moatless_types::user::UserRole;

use super::routes::SseState;

/// Per-connection access context for the firehose, resolved once from the
/// canonical `AuthIdentity` the auth middleware injects.
///
/// `None` means no identity was attached, so the handler rejects the request;
/// `Some` filters non-admin callers per task.
pub(crate) struct SseAuth(Option<ScopeContext>);

impl<S: Send + Sync> FromRequestParts<S> for SseAuth {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = parts.extensions.get::<AuthIdentity>().map(|id| {
            ScopeContext::new(id.user_id.clone().to_string(), id.role == UserRole::Admin)
        });
        Ok(SseAuth(ctx))
    }
}

/// Extract the `taskId` from a serialized `StreamEvent` payload, if present.
fn payload_task_id(payload: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()?
        .get("taskId")?
        .as_str()
        .map(str::to_string)
}

/// Query parameters for the SSE endpoint.
#[derive(Deserialize)]
pub struct SseQuery {
    /// Comma-separated event type filter (e.g., `?types=agent,environment`).
    pub types: Option<String>,
}

/// SSE endpoint handler. Returns an NDJSON stream of raw event lines plus a 30s
/// `{"type":"ping",...}` keepalive.
pub(crate) async fn sse_handler(
    State(state): State<SseState>,
    SseAuth(scope): SseAuth,
    Query(query): Query<SseQuery>,
) -> Response {
    // Fail closed: an unauthenticated caller would otherwise get the
    // unfiltered firehose for every task in the deployment.
    let Some(scope) = scope else {
        return axum::response::IntoResponse::into_response(axum::http::StatusCode::UNAUTHORIZED);
    };

    let rx = state.broadcast.subscribe();
    let cancel = state.cancel.clone();
    let tasks = state.tasks.clone();

    // `None` → admin, no per-task filtering.
    // `Some(ctx)` → filter every event by whether `ctx` can read its task.
    let access: Option<ScopeContext> = Some(scope).filter(|ctx| !ctx.is_admin);

    let type_filters: Vec<String> = query
        .types
        .map_or_default(|t| t.split(',').map(|s| s.trim().to_string()).collect());

    let ndjson_stream = stream! {
        let mut rx = rx;
        // Per-connection task_id → may-read cache, so we hit the DB at most once
        // per distinct task for the life of the stream.
        let mut access_cache: HashMap<String, bool> = HashMap::new();
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

                            // Per-tenant filter: a scoped user only sees events
                            // for tasks it can read. An event whose task can't be
                            // resolved/authorized is dropped (fail closed).
                            if let Some(ref ctx) = access {
                                let Some(task_id) = payload_task_id(&payload) else {
                                    continue;
                                };
                                let allowed = if let Some(&a) = access_cache.get(&task_id) {
                                    a
                                } else {
                                    let a = tasks.may_access(&task_id, ctx).await.unwrap_or(false);
                                    access_cache.insert(task_id, a);
                                    a
                                };
                                if !allowed {
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
                            debug!("SSE broadcast sender closed");
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

    let body = Body::from_stream(ndjson_stream.map(|r| r.map(std::string::String::into_bytes)));

    #[allow(clippy::expect_used)]
    // Response::builder() only fails with invalid header names (ours are static)
    Response::builder()
        .header("Content-Type", "application/x-ndjson")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("X-Accel-Buffering", "no")
        .body(body)
        .expect("valid SSE response with static header names")
}
