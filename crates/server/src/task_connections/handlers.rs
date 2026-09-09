//! Task connection query HTTP adaptation.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use moatless_adapter_model::registry::Adapters;
use moatless_api_schemas::task_connections::{
    Connection, GetTaskConnectionsQuery, ListTaskConnectionsQuery, TaskConnectionsResponse,
};
use moatless_core::http_auth::AuthUser;
use moatless_loops::bindings::operations::Bindings;
use moatless_ports::identity::UserDisplay;
use moatless_ports::workspace::RepositoryReader;
use moatless_tasks::{
    message::operations::Messages, task::error::TaskError, task::operations::TaskCommands,
};
use std::sync::Arc;

/// List tasks with their external connections, statuses, and message counts.
#[utoipa::path(
    get,
    path = "/api/v1/admin/task-connections",
    tag = "tasks",
    params(ListTaskConnectionsQuery),
    responses(
        (status = 200, description = "Tasks with their external connections", body = TaskConnectionsResponse),
        (status = 403, description = "Admin role required", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "HTTP extractors supply the six read ports assembled into TaskConnectionReads"
)]
pub async fn list_task_connections(
    State(tasks): State<Arc<TaskCommands>>,
    State(users): State<Arc<dyn UserDisplay>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    State(task_bindings): State<Arc<Bindings>>,
    State(messages): State<Arc<Messages>>,
    State(adapters): State<Option<Arc<Adapters>>>,
    auth: AuthUser,
    Query(query): Query<ListTaskConnectionsQuery>,
) -> Result<Json<TaskConnectionsResponse>, TaskError> {
    let reads = super::query::TaskConnectionReads {
        tasks: tasks.as_ref(),
        users: users.as_ref(),
        repositories: repositories.as_ref(),
        task_bindings: task_bindings.as_ref(),
        messages: messages.as_ref(),
        adapters: adapters.as_ref(),
    };
    Ok(Json(
        super::query::list_task_connections(reads, &auth.scope_context(), query).await?,
    ))
}

/// Get a single task's external connections, statuses, and message counts.
///
/// Per-task counterpart to [`list_task_connections`]. Unlike the admin list,
/// this is scope-checked like [`crate::task::handlers::get_task`] (not admin-gated),
/// so any user who can see the task can see its connections.
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
