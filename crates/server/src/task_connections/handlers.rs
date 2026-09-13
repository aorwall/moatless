//! Task connection query HTTP adaptation.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use moatless_adapter_model::registry::Adapters;
use moatless_api_schemas::task_connections::{Connection, GetTaskConnectionsQuery};
use moatless_core::http_auth::AuthUser;
use moatless_loops::bindings::operations::Bindings;
use moatless_tasks::{
    message::operations::Messages, task::error::TaskError, task::operations::TaskCommands,
};
use std::sync::Arc;

/// Get a single task's external connections, statuses, and message counts.
///
/// Scope-checked like [`moatless_tasks::task::handlers::get_task`], so any user
/// who can see the task can see its connections.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/connections",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        GetTaskConnectionsQuery
    ),
    responses(
        (status = 200, description = "The task's external connections", body = Vec<Connection>),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_task_connections(
    State(tasks): State<Arc<TaskCommands>>,
    State(task_bindings): State<Arc<Bindings>>,
    State(messages): State<Arc<Messages>>,
    State(adapters): State<Option<Arc<Adapters>>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<GetTaskConnectionsQuery>,
) -> Result<Json<Vec<Connection>>, TaskError> {
    Ok(Json(
        super::query::get_task_connections(
            tasks.as_ref(),
            task_bindings.as_ref(),
            messages.as_ref(),
            adapters.as_ref(),
            &auth.scope_context(),
            task_id,
            query,
        )
        .await?,
    ))
}
