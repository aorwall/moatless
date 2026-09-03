//! SSE endpoint handlers.
//!
//! Serves the live firehose (`GET /api/v1/events/stream`) as an NDJSON stream
//! and the historical task-events read (`GET /api/v1/tasks/{task_id}/events`).
//! The routers that mount these are assembled in [`super::routes`].

use std::collections::HashMap;
use std::sync::Arc;

use async_stream::stream;
use axum::Json;
use axum::body::Body;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::response::Response;
use futures::stream::StreamExt;
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::{debug, warn};

use moatless_types::access_control::ScopeContext;

use crate::extractors::AuthUser;
use crate::task::dao::TaskDao;
use crate::task_event::dao::TaskEventDao;
use crate::task_event::models::TaskEvent;
use moatless_types::auth::AuthIdentity;
use moatless_types::user::UserRole;

use super::routes::SseState;

/// Per-connection access context for the firehose, resolved once from the
/// canonical `AuthIdentity` the auth middleware injects.
///
/// `None` means no identity was attached — the request did not authenticate,
/// and the handler rejects it. When `Some`, non-admin callers are filtered per
/// task.
pub(crate) struct SseAuth(Option<ScopeContext>);

impl<S: Send + Sync> FromRequestParts<S> for SseAuth {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = parts
            .extensions
            .get::<AuthIdentity>()
            .map(|id| ScopeContext {
                user_id: id.user_id.clone().to_string(),
                is_admin: id.role == UserRole::Admin,
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

/// Query parameters for the historical task events endpoint.
#[derive(Deserialize)]
pub struct TaskEventsQuery {
    /// Comma-separated event type filter (e.g., `?types=sandbox,server`).
    pub types: Option<String>,
    /// Maximum number of events to return (default 500, max 2000).
    pub limit: Option<usize>,
}

/// SSE endpoint handler.
///
/// Returns an NDJSON stream with:
/// - Event lines: raw JSON from Redis + `\n`
/// - Ping every 30s: `{"type":"ping","timestamp":"..."}\n`
///
/// The shared broadcast bus carries events for *all* tasks, so each connection
/// filters them to what the authenticated caller may read. Admins receive
/// everything; scoped users receive only events for tasks they can access,
/// resolved once per distinct task and cached for the life of the connection.
/// Unauthenticated callers are rejected outright.
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

    // Parse type filters
    let type_filters: Vec<String> = query
        .types
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

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
                                    let a = match tasks.as_ref() {
                                        Some(repo) => repo
                                            .find_by_id(&task_id, ctx)
                                            .await
                                            .is_ok_and(|t| t.is_some()),
                                        None => false,
                                    };
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

    // Convert to axum body
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

/// Historical task events handler.
///
/// Returns stored events from the `task_events` Postgres table for a given
/// task, allowing the frontend to hydrate the Events tab on page reload.
///
/// History is served from Postgres — not Redis `XRANGE` — because the Redis
/// stream is trimmed by `MAXLEN` and expires after a 72h TTL, silently
/// truncating older history. Every event is durably persisted to `task_events`
/// by the `event_handler` consumer, so Postgres is the complete source of truth.
/// Redis is kept strictly for the live SSE tail (`/api/v1/events/stream`).
///
/// **Seam with the live tail:** the frontend loads this page first, then opens
/// the SSE stream. Because the SSE consumer creates its group at the stream
/// head, an event published between the two calls can appear both here and on
/// the live stream. Each returned envelope carries its `eventId`, so the client
/// dedupes across the boundary by id — no gap, no duplicate rendered.
pub(crate) async fn task_events_handler(
    State(tasks): State<Arc<dyn TaskDao>>,
    State(task_events): State<Arc<dyn TaskEventDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<TaskEventsQuery>,
) -> Json<Vec<serde_json::Value>> {
    // Cross-tenant guard: only return history for a task the caller can access.
    // `find_by_id` bakes the scope filter into SQL and returns `None` when the
    // user has no access — so an unauthorized (or unknown) task id yields an
    // empty list, never another tenant's events.
    match tasks.find_by_id(&task_id, &auth.scope_context()).await {
        Ok(Some(_)) => {}
        Ok(None) => return Json(vec![]),
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to authorize task events access");
            return Json(vec![]);
        }
    }

    let repo = task_events;

    // Page size: default 500, capped at 2000.
    let limit = i64::try_from(query.limit.unwrap_or(500).min(2000)).unwrap_or(500);

    // Comma-separated `types` filter maps to a SQL `type = ANY($types)` filter
    // in `list_recent_events` (an empty/absent filter returns all types).
    let event_types: Option<Vec<String>> = query
        .types
        .map(|t| {
            t.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());

    // Newest-first selection, returned oldest-first: `limit` truncates the head
    // of the log, so a long-running task's page tracks its tail instead of
    // freezing on the window it opened with.
    let rows = match repo
        .list_recent_events(&task_id, &event_types, limit, 0)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to read task events from Postgres");
            return Json(vec![]);
        }
    };

    let events = rows.into_iter().map(event_row_to_json).collect();
    Json(events)
}

/// Reconstruct the SSE `StreamEvent` wire envelope from a persisted row.
///
/// This mirrors `moatless_types::event::StreamEvent`'s serialization exactly (camelCase
/// keys, `type` for the event type, optional `agentType`/`turnNumber` omitted
/// when absent), so the payload is byte-for-byte compatible with what the old
/// Redis `XRANGE` replay produced — the frontend needs no changes.
fn event_row_to_json(row: TaskEvent) -> serde_json::Value {
    // Derived, not stored: the row predates the field, and the answer is a
    // function of `(type, sub_type)` alone, so replay and live stream agree
    // without a migration.
    let working = moatless_types::event::agent_working(&row.event_type, &row.sub_type);

    let mut map = serde_json::Map::new();
    map.insert("eventId".to_string(), row.event_id.into());
    map.insert("type".to_string(), row.event_type.into());
    map.insert("subType".to_string(), row.sub_type.into());
    map.insert("taskId".to_string(), row.task_id.into());
    map.insert("timestamp".to_string(), row.timestamp.into());
    map.insert("isError".to_string(), row.is_error.into());
    if let Some(agent_type) = row.agent_type {
        map.insert("agentType".to_string(), agent_type.into());
    }
    if let Some(turn_number) = row.turn_number {
        map.insert("turnNumber".to_string(), turn_number.into());
    }
    if let Some(working) = working {
        map.insert("agentWorking".to_string(), working.into());
    }
    map.insert("payload".to_string(), row.payload);
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use moatless_tasks::task::dao::MockTaskDao;
    use moatless_tasks::task_event::dao::MockTaskEventDao;
    use moatless_types::access_control::Scope;
    use moatless_types::task::{Task, TaskStatus, Visibility};
    use serde_json::json;

    fn sample_row() -> TaskEvent {
        TaskEvent {
            id: 1,
            event_id: "evt-1".to_string(),
            task_id: "task-1".to_string(),
            event_type: "sandbox".to_string(),
            sub_type: "ready".to_string(),
            is_error: false,
            agent_type: Some("claude".to_string()),
            turn_number: Some(3),
            payload: json!({ "message": "up" }),
            timestamp: "2026-07-12T00:00:00.000Z".to_string(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn maps_row_to_stream_event_envelope() {
        let value = event_row_to_json(sample_row());
        assert_eq!(
            value,
            json!({
                "eventId": "evt-1",
                "type": "sandbox",
                "subType": "ready",
                "taskId": "task-1",
                "timestamp": "2026-07-12T00:00:00.000Z",
                "isError": false,
                "agentType": "claude",
                "turnNumber": 3,
                "payload": { "message": "up" },
            })
        );
    }

    #[test]
    fn omits_absent_optional_fields() {
        let mut row = sample_row();
        row.agent_type = None;
        row.turn_number = None;
        let value = event_row_to_json(row);
        let obj = value.as_object().expect("object");
        assert!(!obj.contains_key("agentType"));
        assert!(!obj.contains_key("turnNumber"));
        // The base envelope the frontend's `isStreamEvent` guard requires.
        assert!(obj.contains_key("eventId"));
        assert!(obj.contains_key("taskId"));
        assert_eq!(obj["type"], json!("sandbox"));
        assert_eq!(obj["subType"], json!("ready"));
        assert_eq!(obj["isError"], json!(false));
        assert!(obj.contains_key("payload"));
    }

    /// The task the scoped read hands back — the handler only cares that it
    /// resolves, so every field beyond the id is filler.
    fn task_fixture() -> Task {
        Task {
            task_id: "task-1".to_string(),
            short_id: None,
            created_at: Utc::now(),
            last_message_at: Utc::now(),
            agent_type: moatless_types::agent::AgentType("claude-code".to_string()),
            repository_id: "repo-1".to_string(),
            status: TaskStatus::Open,
            total_cost_usd: 0.0,
            site_url: None,
            template_id: None,
            vibe: None,
            name: None,
            description: None,
            site_id: None,
            site_provider: None,
            branch: None,
            model: None,
            created_by: None,
            visibility: Visibility::Private,
            scope: Scope::User,
            owner_user_id: None,
            updated_at: None,
            append_system_prompt: None,
            headless: false,
            ttl_minutes: 0,
            inactive_ttl_days: 0,
            loop_id: None,
            workspace_id: None,
            sandbox_id: None,
            checkout_mode: None,
            skills: Vec::new(),
        }
    }

    /// Task repo whose scoped `find_by_id` lets the caller through.
    fn tasks_allowing_access() -> Arc<dyn TaskDao> {
        let mut tasks = MockTaskDao::new();
        tasks
            .expect_find_by_id()
            .returning(|_, _| Ok(Some(task_fixture())));
        Arc::new(tasks)
    }

    /// `count` events named `evt-1..evt-count`, oldest first — a task's log.
    fn event_log(count: i64) -> Vec<TaskEvent> {
        (1..=count)
            .map(|id| TaskEvent {
                id,
                event_id: format!("evt-{id}"),
                ..sample_row()
            })
            .collect()
    }

    fn auth_user() -> AuthUser {
        AuthUser {
            user_id: uuid::Uuid::new_v4(),
            login: "reader".to_string(),
            role: Some("user".to_string()),
            provider: Some("local".to_string()),
            group_id: None,
            task_id: None,
            is_agent_context: false,
        }
    }

    #[tokio::test]
    async fn serves_the_newest_events_when_the_log_is_longer_than_the_limit() {
        let mut events = MockTaskEventDao::new();
        // Stands in for the DAO's contract: the newest `limit` rows, handed back
        // oldest-first. A handler reading from the start of the log instead
        // would have to ask for something this double does not answer.
        events
            .expect_list_recent_events()
            .withf(|task_id, types, limit, offset| {
                task_id == "task-1" && types.is_none() && *limit == 2 && *offset == 0
            })
            .returning(|_, _, limit, _| {
                let log = event_log(5);
                let skip = log.len().saturating_sub(usize::try_from(limit).unwrap());
                Ok(log.into_iter().skip(skip).collect())
            });

        let Json(body) = task_events_handler(
            State(tasks_allowing_access()),
            State(Arc::new(events) as Arc<dyn TaskEventDao>),
            auth_user(),
            Path("task-1".to_string()),
            Query(TaskEventsQuery {
                types: None,
                limit: Some(2),
            }),
        )
        .await;

        let ids: Vec<&str> = body
            .iter()
            .map(|e| e["eventId"].as_str().expect("eventId"))
            .collect();
        assert_eq!(ids, ["evt-4", "evt-5"]);
    }
}
