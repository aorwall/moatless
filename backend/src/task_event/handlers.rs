//! Task event HTTP handlers.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;

use super::dao::TaskEventDao;
use super::error::TaskEventError;
use super::schemas::*;
use crate::extractors::AuthUser;
use crate::task::dao::TaskDao;

/// List persisted events for a task.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/events/history",
    tag = "task-events",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ListTaskEventsQuery,
    ),
    responses(
        (status = 200, description = "List of task events", body = ListTaskEventsResponse),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn list_task_events(
    State(tasks): State<Arc<dyn TaskDao>>,
    State(task_events): State<Arc<dyn TaskEventDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<ListTaskEventsQuery>,
) -> Result<Json<ListTaskEventsResponse>, TaskEventError> {
    // Verify task exists and user has access
    tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskEventError::TaskNotFound(task_id.clone()))?;

    let limit = query.limit.unwrap_or(500).min(5000);
    let offset = query.offset.unwrap_or(0);

    let event_types: Option<Vec<String>> = query.types.map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

    // Newest-first selection, returned oldest-first: the UI's Events tab and
    // `moat events` both want the tail of a task's history, and `list_events`
    // would hand back the oldest `limit` rows — so a long-running task showed a
    // window frozen near its start.
    let events = task_events
        .list_recent_events(&task_id, &event_types, limit, offset)
        .await?;

    let items = events
        .into_iter()
        .map(|e| TaskEventResponse {
            event_id: e.event_id,
            task_id: e.task_id,
            event_type: e.event_type,
            sub_type: e.sub_type,
            is_error: e.is_error,
            agent_type: e.agent_type,
            turn_number: e.turn_number,
            payload: e.payload,
            timestamp: e.timestamp,
        })
        .collect();

    Ok(Json(ListTaskEventsResponse { items }))
}
