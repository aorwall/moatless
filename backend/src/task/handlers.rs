//! Task HTTP handlers — coordinate repository calls + enrichment.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{Path, Query, State};

use moatless_events::{SandboxEventPayload, SandboxStatus, TaskSubType};

use super::dao::{TaskFilter, TaskPatch};
use super::models::Task;
use super::relations::TaskRelationType;
use crate::extractors::AuthUser;
use crate::sandbox::lifecycle::ArchiveRetention;
use crate::state::AppState;
use crate::task::models::{SandboxIntentView, TaskStatus};
use moatless_backend_core::scope::{ScopeContext, ScopedResource};

use crate::repository::models::RepositoryProvider;

use crate::bindings::dao::{TaskBindingDao, get_bindings_for_tasks};
use crate::bindings::schemas::TaskBindingSummary;

use super::error::TaskError;
use super::schemas::*;
use super::service;

/// Create a new task.
#[utoipa::path(
    post,
    path = "/api/v1/tasks",
    tag = "tasks",
    request_body = CreateTaskRequest,
    responses(
        (status = 200, description = "Task created", body = CreateTaskResponse),
        (status = 400, description = "Invalid request or unusable Codex credential", body = crate::error::ErrorBody),
        (status = 403, description = "GitHub not connected, or no access to the repository", body = crate::error::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn create_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateTaskRequest>,
) -> Result<Json<CreateTaskResponse>, TaskError> {
    // enrich_task is DB-heavy (~6 queries); enrich exactly once. The event
    // closure stashes the enriched response here and the HTTP response reuses
    // it, avoiding both a second round of queries and an event/response
    // divergence window.
    let user_id = auth.user_id.to_string();
    let enriched: Arc<Mutex<Option<TaskResponse>>> = Arc::new(Mutex::new(None));

    let state_ev = state.clone();
    let uid_ev = user_id.clone();
    let enriched_ev = enriched.clone();
    let outcome = service::create_task(
        &state,
        &auth.scope_context(),
        &user_id,
        service::CreateTaskIntent::new(req),
        move |task| async move {
            match enrich_task(&task, &state_ev, &uid_ev).await {
                Ok(resp) => {
                    let payload = serde_json::json!({ "task": resp });
                    if let Ok(mut cell) = enriched_ev.lock() {
                        *cell = Some(resp);
                    }
                    payload
                }
                Err(e) => {
                    // Fall back to the un-enriched row projection so the
                    // payload keeps the same `{"task": TaskResponse}` shape
                    // consumers destructure (tags/bindings/relations/sandbox
                    // empty rather than a differently-shaped object).
                    tracing::warn!(task_id = %task.task_id, error = %e, "Failed to enrich created task for event; using unenriched row");
                    let resp = build_task_response(
                        &task,
                        &SandboxIntentView::default(),
                        Vec::new(),
                        ResolvedNames::default(),
                        Vec::new(),
                        Vec::new(),
                    );
                    let payload = serde_json::json!({ "task": resp });
                    if let Ok(mut cell) = enriched_ev.lock() {
                        *cell = Some(resp);
                    }
                    payload
                }
            }
        },
    )
    .await?;

    // Reuse the response the event closure produced (enriched, or its
    // un-enriched fallback). Only re-enrich when the closure never ran because
    // events are disabled, so nothing was stashed.
    let stashed = enriched.lock().ok().and_then(|mut cell| cell.take());
    let task_response = match stashed {
        Some(resp) => resp,
        None => enrich_task(&outcome.task, &state, &user_id).await?,
    };
    Ok(Json(CreateTaskResponse {
        task_id: outcome.task.task_id.clone(),
        status: outcome.task.status,
        task: task_response,
    }))
}

/// List tasks with pagination and filters.
#[utoipa::path(
    get,
    path = "/api/v1/tasks",
    tag = "tasks",
    params(ListTasksQuery),
    responses(
        (status = 200, description = "Paginated task list", body = ListTasksResponse)
    )
)]
pub async fn list_tasks(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<ListTasksResponse>, TaskError> {
    let limit = query.limit.unwrap_or(50).min(200);
    let offset = query.offset.unwrap_or(0);

    let filter = TaskFilter {
        scope: auth.scope_context(),
        limit,
        offset,
        status: query.status,
        exclude_status: None,
        repository_id: query.repository_id.clone(),
        created_by: query.created_by.clone(),
        tag: query.tag.clone(),
        parent_task_id: query.parent_task_id.clone(),
        followed_by: query.followed_by.clone(),
    };

    let (rows, total) = state.tasks.list_tasks(&filter).await?;

    let task_ids: Vec<String> = rows.iter().map(|r| r.task_id.clone()).collect();
    let all_tags = state.tags.get_tags_for_tasks(&task_ids).await?;

    // Durable Sandbox state comes from the `sandboxes` table; absent rows fall
    // back to the lazy converged-Stopped default in the loop below.
    let sandbox_intents = state
        .tasks
        .get_sandbox_intents(&task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to load sandbox intents for task list");
            Default::default()
        });

    // Batch resolve user display names (single query instead of N+1)
    let creator_ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r.created_by.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let user_names = resolve_user_display_names_batch(&state, &creator_ids).await;

    // Batch resolve repository names (single query instead of N+1)
    let repo_ids: Vec<String> = rows
        .iter()
        .map(|r| r.repository_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let repo_names = resolve_repo_names_batch(&state, &repo_ids).await;

    // Batch resolve workspace names for the tasks that have a workspace.
    let workspace_ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r.workspace_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let workspace_names = resolve_workspace_names_batch(&state, &workspace_ids).await;

    // Batch resolve outgoing relations for tasks (single query instead of N+1).
    let all_relations = load_outgoing_relations_batch(&state, &task_ids).await;

    // Batch resolve external bindings for tasks (single query instead of N+1)
    let all_bindings = get_bindings_for_tasks(&state.db, &task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve task bindings");
            vec![]
        });

    // Batch resolve the requesting user's read markers (single query). Drives
    // the unread indicator; best-effort — empty on error.
    let read_receipts = state
        .tasks
        .get_read_receipts(&auth.user_id.to_string(), &task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve read receipts");
            Default::default()
        });

    // Batch resolve which of these tasks the requesting user follows (single
    // query). Drives the follow indicator; best-effort — empty on error.
    let followed = state
        .tasks
        .get_followed(&auth.user_id.to_string(), &task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve follow state");
            Default::default()
        });

    let items: Vec<TaskResponse> = rows
        .iter()
        .map(|row| {
            let tags: Vec<TagResponse> = all_tags
                .iter()
                .filter(|t| t.task_id == row.task_id)
                .map(|t| TagResponse {
                    tag_id: t.tag_id.clone(),
                    name: t.name.clone(),
                })
                .collect();

            let display_name = row
                .created_by
                .as_ref()
                .and_then(|uid| user_names.get(uid))
                .cloned();

            let repo_name = repo_names.get(&row.repository_id).cloned();
            let workspace_name = row
                .workspace_id
                .as_ref()
                .and_then(|wid| workspace_names.get(wid))
                .cloned();

            let bindings: Vec<TaskBindingSummary> = all_bindings
                .iter()
                .filter(|c| c.task_id == row.task_id)
                .map(|c| TaskBindingSummary {
                    loop_id: c.loop_id.clone(),
                    adapter_kind: crate::adapters::common::parse_adapter_kind(&c.adapter_kind),
                    source_id: c.source_id.clone(),
                    source_name: c.source_name.clone(),
                    thread_id: None,
                    external_thread_id: None,
                    subject_key: None,
                    binding_id: None,
                    origin: None,
                    last_event_at: None,
                })
                .collect();

            let relations = all_relations.get(&row.task_id).cloned().unwrap_or_default();
            let sandbox = sandbox_intents
                .get(&row.task_id)
                .cloned()
                .unwrap_or_default();

            let mut resp = build_task_response(
                row,
                &sandbox,
                tags,
                ResolvedNames {
                    creator: display_name,
                    repository: repo_name,
                    workspace: workspace_name,
                },
                bindings,
                relations,
            );
            resp.last_read_at = read_receipts
                .get(&row.task_id)
                .map(chrono::DateTime::to_rfc3339);
            resp.is_following = Some(followed.contains(&row.task_id));
            resp
        })
        .collect();

    Ok(Json(ListTasksResponse {
        items,
        pagination: PaginationResponse {
            limit,
            offset,
            total,
            has_more: offset + limit < total,
        },
    }))
}

/// Get a task by ID.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task details", body = TaskResponse),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<TaskResponse>, TaskError> {
    let row = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&row, &state, &auth.user_id.to_string()).await?;
    Ok(Json(resp))
}

/// Get conversation statistics for a task.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/statistics",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task conversation statistics", body = TaskStatisticsResponse),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_task_statistics(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<TaskStatisticsResponse>, TaskError> {
    let row = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let (messages, _, activities) = state.messages.get_all(&task_id).await?;
    let turn_metrics = state
        .task_turns
        .list_turn_metrics(&task_id)
        .await
        .unwrap_or_default();
    Ok(Json(super::statistics::build_task_statistics(
        &row,
        &messages,
        &activities,
        &turn_metrics,
    )))
}

/// Update task name and description.
#[utoipa::path(
    patch,
    path = "/api/v1/tasks/{task_id}",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = UpdateTaskMetadataRequest,
    responses(
        (status = 200, description = "Task updated", body = TaskResponse),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn update_task_metadata(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskMetadataRequest>,
) -> Result<Json<TaskResponse>, TaskError> {
    if let Some(ref name) = req.name {
        service::validate_task_name(name)?;
    }

    let scope_ctx = auth.scope_context();
    let updated = state
        .tasks
        .update_task(
            &task_id,
            &TaskPatch {
                name: req.name.clone(),
                description: req.description.clone(),
                ..Default::default()
            },
            &scope_ctx,
        )
        .await?;
    if !updated {
        return Err(TaskError::NotFound(task_id.clone()));
    }
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task metadata updated");

    let row = state
        .tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                &task_id,
                TaskSubType::MetadataUpdated,
                Some(row.agent_type.as_str()),
                serde_json::json!({
                    "name": req.name,
                    "description": req.description,
                }),
            )
            .await;
    }

    let resp = enrich_task(&row, &state, &auth.user_id.to_string()).await?;
    Ok(Json(resp))
}

/// Deactivate all of a task's external bindings (best-effort).
///
/// Called when a task is closed or deleted. A closed task is terminal, so its
/// adapter "listeners" (Linear issue / GitHub PR / Slack thread bindings) must
/// stop routing inbound events — otherwise a new comment would re-provision the
/// sandbox and resurrect the task. Logs on failure but never blocks the
/// close/delete; the runtime guards in `ensure_sandbox_and_notify` and
/// `send_message_flow` are the backstop if this doesn't run.
async fn deactivate_task_bindings(bindings: &dyn TaskBindingDao, task_id: &str) {
    match bindings.deactivate_by_task(task_id).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(task_id, deactivated = n, "Deactivated task bindings"),
        Err(e) => {
            tracing::warn!(task_id, error = %e, "Failed to deactivate task bindings");
        }
    }
}

/// Delete a task and cleanup its workspace.
#[utoipa::path(
    delete,
    path = "/api/v1/tasks/{task_id}",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Task deleted"),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    let scope_ctx = auth.scope_context();

    // Permanent deletion is strict remove with archive deletion, and the row is
    // deleted only on success. The lifecycle handle owns the graceful agent stop
    // and awaits observed absence; on failure the typed error surfaces, the row
    // survives with `Removed` intent, and the reconciler keeps retrying.
    state
        .sandbox
        .remove(&task_id, ArchiveRetention::DeleteArchives)
        .await?;

    // Stop external listeners. `task_bindings.task_id` is not an FK, so rows
    // don't cascade on delete — deactivate them so inbound routing stops.
    deactivate_task_bindings(state.task_bindings.as_ref(), &task_id).await;

    let deleted = state.tasks.delete_task(&task_id, &scope_ctx).await?;
    if !deleted {
        return Err(TaskError::NotFound(task_id.clone()));
    }
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task deleted");

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Mark a task's conversation as read for the requesting user, up to now.
/// Clears the unread indicator; safe to call repeatedly (the stored marker
/// only ever advances).
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/read",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Task marked as read"),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn mark_task_read(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    // Ensure the task exists and is visible to the user before recording a read.
    state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    state
        .tasks
        .mark_task_read(&auth.user_id.to_string(), &task_id, chrono::Utc::now())
        .await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Follow a task so it appears in the requesting user's personal task list.
/// Any task the user can see may be followed. Idempotent.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/follow",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task followed", body = FollowTaskResponse),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn follow_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<FollowTaskResponse>, TaskError> {
    // Ensure the task exists and is visible to the user before following.
    state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    state
        .tasks
        .follow_task(&auth.user_id.to_string(), &task_id)
        .await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task followed");

    Ok(Json(FollowTaskResponse { following: true }))
}

/// Stop following a task, removing it from the requesting user's personal task
/// list. Idempotent.
#[utoipa::path(
    delete,
    path = "/api/v1/tasks/{task_id}/follow",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task unfollowed", body = FollowTaskResponse),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn unfollow_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<FollowTaskResponse>, TaskError> {
    // Ensure the task exists and is visible to the user before unfollowing.
    state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    state
        .tasks
        .unfollow_task(&auth.user_id.to_string(), &task_id)
        .await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task unfollowed");

    Ok(Json(FollowTaskResponse { following: false }))
}

/// Close a task and stop its sandbox.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/close",
    tag = "tasks",
    request_body = Option<CloseTaskRequest>,
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task closed", body = TaskResponse),
        (status = 400, description = "Child-close choice required", body = crate::error::ErrorBody),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn close_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    req: Option<Json<CloseTaskRequest>>,
) -> Result<Json<TaskResponse>, TaskError> {
    let req = req.map(|Json(req)| req).unwrap_or_default();
    let scope_ctx = auth.scope_context();
    // Read-check + control-check: find_by_id uses access_filter (visibility);
    // the inline can_control check enforces owner/admin policy without the
    // deleted ensure_can_control_task helper.
    let row = state
        .tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    if !scope_ctx.can_control(&task_resource(&row)) {
        return Err(TaskError::NotFound(task_id.clone())); // existence hiding
    }

    let children = state
        .task_relations
        .list_incoming(&task_id, Some(TaskRelationType::CreatedBy))
        .await
        .map_err(TaskError::Db)?;
    if !children.is_empty() && req.close_children.is_none() {
        return Err(TaskError::BadRequest(
            "closeChildren must be explicitly set to true or false when closing a task with children"
                .to_string(),
        ));
    }

    if req.close_children == Some(true) {
        let mut visited = HashSet::new();
        close_task_tree(&state, &task_id, &mut visited).await?;
    } else {
        close_single_task(&state, &task_id).await?;
    }
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task closed");

    let updated = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&updated, &state, &auth.user_id.to_string()).await?;
    Ok(Json(resp))
}

async fn close_task_tree(
    state: &AppState,
    task_id: &str,
    visited: &mut HashSet<String>,
) -> Result<(), TaskError> {
    let mut stack = vec![task_id.to_string()];
    let mut close_order = Vec::new();

    while let Some(current_task_id) = stack.pop() {
        if !visited.insert(current_task_id.clone()) {
            continue;
        }

        match state
            .task_relations
            .list_incoming(&current_task_id, Some(TaskRelationType::CreatedBy))
            .await
        {
            Ok(relations) => {
                for relation in relations {
                    stack.push(relation.from_task_id);
                }
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %current_task_id,
                    error = %e,
                    "Failed to list child tasks during close cascade"
                );
                return Err(TaskError::Db(e));
            }
        }

        close_order.push(current_task_id);
    }

    for current_task_id in close_order.into_iter().rev() {
        close_single_task(state, &current_task_id).await?;
    }

    Ok(())
}

async fn close_single_task(state: &AppState, task_id: &str) -> Result<(), TaskError> {
    // reason: close cascade reaches child tasks owned by other users; system ctx
    // is required to traverse the full tree without re-checking per-child ownership.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();

    let Some(row) = state.tasks.find_by_id(task_id, &sys_ctx).await? else {
        tracing::warn!(task_id, "close cascade: task vanished mid-close");
        return Ok(());
    };

    if row.status == TaskStatus::Closed {
        return Ok(());
    }

    // Persist `Closed` before removing the sandbox — the Closed guard permits
    // remove, and lifecycle owns the sandbox intent plus the graceful agent stop.
    state
        .tasks
        .update_task(
            task_id,
            &TaskPatch {
                status: Some(TaskStatus::Closed),
                ..Default::default()
            },
            &sys_ctx,
        )
        .await?;

    // Remove the sandbox (best-effort on close). Preserve S3 archives when
    // configured so the task can later be revived via reopen; a failure leaves
    // the `Removed` intent for the reconciler.
    let retention = ArchiveRetention::on_close(state.config.sandbox.retain_storage_on_close);
    if let Err(e) = state.sandbox.remove(task_id, retention).await {
        tracing::warn!(task_id, error = %e, "Failed to remove sandbox on close");
    }

    // A closed task is terminal — stop its external listeners so inbound
    // comments don't resurrect it.
    deactivate_task_bindings(state.task_bindings.as_ref(), task_id).await;

    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                task_id,
                TaskSubType::Closed,
                Some(row.agent_type.as_str()),
                serde_json::json!({}),
            )
            .await;
    }

    Ok(())
}

/// Reopen a previously closed task.
///
/// Flips a `Closed` task back to `Open` so it can be started and chatted with
/// again. No sandbox is provisioned here — that happens on the next `start` or
/// inbound message, at which point the sandbox restores any retained S3 state
/// (Claude transcript + `s3`-mode workspace) keyed by `task_id`. Whether that
/// state still exists depends on `SANDBOX_RETAIN_STORAGE_ON_CLOSE` having been
/// set when the task was closed; if not, the task reopens with a fresh
/// sandbox. External bindings deactivated on close are left inactive.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/reopen",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task reopened", body = TaskResponse),
        (status = 400, description = "Task is not closed", body = crate::error::ErrorBody),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn reopen_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<TaskResponse>, TaskError> {
    let scope_ctx = auth.scope_context();
    let row = state
        .tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    if !scope_ctx.can_control(&task_resource(&row)) {
        return Err(TaskError::NotFound(task_id.clone())); // existence hiding
    }

    if row.status != TaskStatus::Closed {
        return Err(TaskError::BadRequest(format!(
            "Cannot reopen task in '{}' status (only closed tasks can be reopened)",
            row.status
        )));
    }

    let updated = state
        .tasks
        .update_task(
            &task_id,
            &TaskPatch {
                status: Some(TaskStatus::Open),
                ..Default::default()
            },
            &scope_ctx,
        )
        .await?;
    if !updated {
        return Err(TaskError::NotFound(task_id.clone()));
    }

    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task reopened");

    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                &task_id,
                TaskSubType::Open,
                Some(row.agent_type.as_str()),
                serde_json::json!({"reason": "reopened"}),
            )
            .await;
    }

    let updated = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&updated, &state, &auth.user_id.to_string()).await?;
    Ok(Json(resp))
}

/// Start a task (provision sandbox and begin agent).
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/start",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Task started"),
        (status = 400, description = "Invalid task state", body = crate::error::ErrorBody),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn start_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    let scope_ctx = auth.scope_context();
    let row = state
        .tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    if !scope_ctx.can_control(&task_resource(&row)) {
        return Err(TaskError::NotFound(task_id.clone())); // existence hiding
    }

    if row.status != TaskStatus::Open && row.status != TaskStatus::Error {
        return Err(TaskError::BadRequest(format!(
            "Cannot start task in '{}' status",
            row.status
        )));
    }

    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Starting task");

    // Pre-flight: validate the GitHub token + repo access before any
    // sandbox resources are created. On failure: mark Error, publish a
    // sandbox.error event, and return 400.
    let repo = state
        .repositories
        .find_by_id(&row.repository_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(row.repository_id.clone()))?;

    let validator = crate::sandbox::preflight::RealGithubValidator;
    let preflight_creds = crate::sandbox::preflight::PreflightCredentials::from_state(&state);
    if let Err(e) = crate::sandbox::preflight::preflight_validate_github_token(
        &preflight_creds,
        &row,
        &repo,
        &validator,
    )
    .await
    {
        let msg = e.user_message();
        tracing::warn!(task_id = %task_id, error = %e, "Start preflight failed");
        if let Err(db_err) = state
            .tasks
            .update_task(
                &task_id,
                &TaskPatch {
                    status: Some(TaskStatus::Error),
                    ..Default::default()
                },
                &scope_ctx,
            )
            .await
        {
            tracing::error!(
                task_id = %task_id,
                error = %db_err,
                "Failed to mark task Error after preflight failure"
            );
        }
        if let Some(ref events) = state.events {
            events
                .publish_sandbox_event_with_payload(
                    &task_id,
                    SandboxStatus::Error,
                    &SandboxEventPayload {
                        message: msg.clone(),
                        error: Some(e.to_string()),
                        lifecycle_event: Some("preflight_failed".to_string()),
                        ..Default::default()
                    },
                )
                .await;
        }
        return Err(TaskError::BadRequest(msg));
    }

    // Best-effort: if the repo ships a `.moatless/` directory, reconcile its
    // workspace config before provisioning. Loops are never synchronized as a
    // side effect of task start.
    maybe_sync_repo_config_on_start(&state, &auth, &repo).await;

    state.sandbox.start(&task_id).await?;

    tracing::info!(task_id = %task_id, "Sandbox start initiated (initializing)");
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Best-effort auto-sync of a repo's workspace config when a task
/// starts. Gated on the repo actually shipping a `.moatless/` directory so we
/// don't hit the provider API for repos that don't use it. Every failure is
/// logged and swallowed — this must never block or fail a task start.
async fn maybe_sync_repo_config_on_start(
    state: &AppState,
    auth: &AuthUser,
    repo: &crate::repository::models::Repository,
) {
    let Some(remote_url) = repo.remote_url.as_deref() else {
        return;
    };
    let provider = repo.provider.unwrap_or(RepositoryProvider::Github);

    // Resolve a token for the existence probe; fall back to an unauthenticated
    // probe (works for public repos) if we can't load one.
    let token = if provider == RepositoryProvider::Github {
        crate::repository::access::resolve_repository_github_token(
            state,
            repo,
            remote_url,
        )
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(repository_id = %repo.id, error = %e, "Config auto-sync: failed to resolve GitHub token; probing unauthenticated");
                None
            })
    } else {
        match state
            .provider_tokens
            .find_provider_token(&auth.user_id.to_string(), provider.as_str())
            .await
        {
            Ok(token) => token.map(|t| t.access_token),
            Err(e) => {
                tracing::warn!(repository_id = %repo.id, error = %e, "Config auto-sync: failed to load provider token; probing unauthenticated");
                None
            }
        }
    };
    let token_str = token.as_deref();
    let branch = repo.default_branch.as_deref();

    if !crate::repository::config_reader::moatless_dir_exists(
        remote_url, branch, token_str, provider,
    )
    .await
    {
        tracing::debug!(repository_id = %repo.id, "Config auto-sync: no .moatless directory, skipping");
        return;
    }

    // Automatic sync: workspaces the user overrode from the UI/CLI stay
    // overridden — only an explicit sync-config or reset-git reclaims them.
    let force = crate::workspace::git_sync::SyncForce::RespectOverrides;
    match crate::workspace::git_sync::sync_repo_workspaces(state, repo, auth, &force).await {
        Ok(workspaces) => {
            tracing::info!(
                repository_id = %repo.id,
                workspaces_created = workspaces.created,
                workspaces_updated = workspaces.updated,
                workspaces_overridden = workspaces.skipped.len(),
                "Workspace config auto-sync on task start complete"
            );
        }
        Err(e) => {
            tracing::warn!(repository_id = %repo.id, error = %e, "Workspace config auto-sync on task start failed");
        }
    }
}

/// Stop the agent running on a task.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/stop",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Agent stopped"),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn stop_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    service::stop_task(&state, &auth.scope_context(), &task_id).await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Set tags for a task.
#[utoipa::path(
    put,
    path = "/api/v1/tasks/{task_id}/tags",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = SetTagsRequest,
    responses(
        (status = 200, description = "Tags updated", body = Vec<TagResponse>),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn set_task_tags(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SetTagsRequest>,
) -> Result<Json<Vec<TagResponse>>, TaskError> {
    let scope_ctx = auth.scope_context();
    let existing = state
        .tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    if !scope_ctx.can_control(&task_resource(&existing)) {
        return Err(TaskError::NotFound(task_id.clone())); // existence hiding
    }

    let tags = state.tags.find_or_create_tags(&req.tags).await?;
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();
    state.tags.set_task_tags(&task_id, &tag_ids).await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, tag_count = tags.len(), "Task tags updated");

    let response: Vec<TagResponse> = tags
        .iter()
        .map(|t| TagResponse {
            tag_id: t.tag_id.clone(),
            name: t.name.clone(),
        })
        .collect();

    Ok(Json(response))
}

/// Update task visibility (public/private).
#[utoipa::path(
    put,
    path = "/api/v1/tasks/{task_id}/visibility",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = UpdateVisibilityRequest,
    responses(
        (status = 200, description = "Visibility updated", body = TaskResponse),
        (status = 400, description = "Invalid visibility value", body = crate::error::ErrorBody),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn update_visibility(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateVisibilityRequest>,
) -> Result<Json<TaskResponse>, TaskError> {
    let scope_ctx = auth.scope_context();
    let updated = state
        .tasks
        .update_task(
            &task_id,
            &TaskPatch {
                visibility: Some(req.visibility),
                ..Default::default()
            },
            &scope_ctx,
        )
        .await?;
    if !updated {
        return Err(TaskError::NotFound(task_id.clone()));
    }
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, visibility = %req.visibility, "Task visibility updated");

    let row = state
        .tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&row, &state, &auth.user_id.to_string()).await?;
    Ok(Json(resp))
}

/// Update a task's TTLs. `ttl_minutes` is the idle TTL (`0` disables the idle
/// reaper — the sandbox stays alive until explicitly stopped). `inactive_ttl_days`
/// is the inactivity TTL (`0` disables auto-close). Both fields are optional;
/// omit a field to leave that TTL unchanged. At least one must be provided.
#[utoipa::path(
    put,
    path = "/api/v1/tasks/{task_id}/ttl",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = UpdateTaskTtlRequest,
    responses(
        (status = 200, description = "TTL updated", body = TaskResponse),
        (status = 400, description = "Invalid TTL value", body = crate::error::ErrorBody),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn update_ttl(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskTtlRequest>,
) -> Result<Json<TaskResponse>, TaskError> {
    if req.ttl_minutes.is_none() && req.inactive_ttl_days.is_none() {
        return Err(TaskError::BadRequest(
            "provide ttl_minutes and/or inactive_ttl_days".to_string(),
        ));
    }
    if req.ttl_minutes.is_some_and(|v| v < 0) {
        return Err(TaskError::BadRequest(
            "ttl_minutes must be >= 0".to_string(),
        ));
    }
    if req.inactive_ttl_days.is_some_and(|v| v < 0) {
        return Err(TaskError::BadRequest(
            "inactive_ttl_days must be >= 0".to_string(),
        ));
    }

    let scope_ctx = auth.scope_context();
    let updated = state
        .tasks
        .update_task(
            &task_id,
            &TaskPatch {
                ttl_minutes: req.ttl_minutes,
                inactive_ttl_days: req.inactive_ttl_days,
                ..Default::default()
            },
            &scope_ctx,
        )
        .await?;
    if !updated {
        return Err(TaskError::NotFound(task_id.clone()));
    }
    tracing::info!(
        task_id = %task_id,
        user_id = %auth.user_id,
        ttl_minutes = ?req.ttl_minutes,
        inactive_ttl_days = ?req.inactive_ttl_days,
        "Task TTL updated"
    );

    let row = state
        .tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&row, &state, &auth.user_id.to_string()).await?;
    Ok(Json(resp))
}

// =========================================================================
// Helpers
// =========================================================================

/// Enrich a single task row into a full response with tags, repo name, display name, sandbox snapshot.
async fn enrich_task(
    row: &Task,
    state: &AppState,
    user_id: &str,
) -> Result<TaskResponse, TaskError> {
    let tags = state.tags.get_tags_for_task(&row.task_id).await?;
    let tag_responses: Vec<TagResponse> = tags
        .iter()
        .map(|t| TagResponse {
            tag_id: t.tag_id.clone(),
            name: t.name.clone(),
        })
        .collect();

    // reason: display enrichment of a task the caller already read; the
    // task's configured repository name is part of that task's own data.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    let repo_name = state
        .repositories
        .find_by_id(&row.repository_id, &sys_ctx)
        .await?
        .map(|r| r.name);

    // Same reasoning as `repo_name` above — labelling the workspace the caller
    // already has access to via the task.
    let workspace_name = match &row.workspace_id {
        Some(wid) => state
            .workspaces
            .find_names_by_ids(std::slice::from_ref(wid))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, workspace_id = %wid, "Failed to resolve workspace name");
                Vec::new()
            })
            .into_iter()
            .next()
            .map(|(_, name)| name),
        None => None,
    };

    let display_name = if let Some(ref uid) = row.created_by {
        resolve_single_user_display_name(state, uid).await
    } else {
        None
    };

    // Resolve external bindings for this task.
    let bindings = resolve_task_bindings(state, &row.task_id).await;

    // Resolve outgoing relations for this task (best-effort — empty on error).
    let relations = match state.task_relations.list_outgoing(&row.task_id).await {
        Ok(rows) => relations_to_dtos(rows),
        Err(e) => {
            tracing::warn!(error = %e, task_id = %row.task_id, "Failed to load task relations");
            Vec::new()
        }
    };

    // Durable Sandbox state comes from the `sandboxes` table; absence is the
    // lazy converged-Stopped default.
    let sandbox = match state
        .tasks
        .get_sandbox_intents(std::slice::from_ref(&row.task_id))
        .await
    {
        Ok(mut intents) => intents.remove(&row.task_id).unwrap_or_default(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                task_id = %row.task_id,
                "Failed to load Sandbox lifecycle intent for task response"
            );
            SandboxIntentView::default()
        }
    };
    let mut resp = build_task_response(
        row,
        &sandbox,
        tag_responses,
        ResolvedNames {
            creator: display_name,
            repository: repo_name,
            workspace: workspace_name,
        },
        bindings,
        relations,
    );

    // Resolve the requesting user's read marker (best-effort — leave None on
    // failure). Drives the unread indicator in clients.
    match state
        .tasks
        .get_read_receipts(user_id, std::slice::from_ref(&row.task_id))
        .await
    {
        Ok(receipts) => {
            resp.last_read_at = receipts.get(&row.task_id).map(chrono::DateTime::to_rfc3339);
        }
        Err(e) => {
            tracing::warn!(error = %e, task_id = %row.task_id, "Failed to load read receipt");
        }
    }

    // Resolve whether the requesting user follows this task (best-effort —
    // leave None on failure). Drives the follow/unfollow toggle in clients.
    match state
        .tasks
        .get_followed(user_id, std::slice::from_ref(&row.task_id))
        .await
    {
        Ok(followed) => {
            resp.is_following = Some(followed.contains(&row.task_id));
        }
        Err(e) => {
            tracing::warn!(error = %e, task_id = %row.task_id, "Failed to load follow state");
        }
    }

    Ok(resp)
}

/// Build a TaskResponse from a row plus pre-resolved tags, names, bindings,
/// and outgoing relations. `parent_task_id` is derived from the first
/// `created_by` relation in `relations` for backward compatibility with
/// clients that pre-date the relations field.
/// Display names resolved for the foreign keys on a task row, so the response
/// can label them without the client re-querying. Each is `None` when the
/// referenced row is absent or its lookup failed; clients fall back to the id.
#[derive(Default)]
struct ResolvedNames {
    creator: Option<String>,
    repository: Option<String>,
    workspace: Option<String>,
}

fn build_task_response(
    row: &Task,
    sandbox: &SandboxIntentView,
    tags: Vec<TagResponse>,
    names: ResolvedNames,
    bindings: Vec<TaskBindingSummary>,
    relations: Vec<TaskRelationDto>,
) -> TaskResponse {
    let title = derive_title(row);

    let parent_task_id = relations
        .iter()
        .find(|r| matches!(r.relation_type, TaskRelationType::CreatedBy))
        .map(|r| r.to_task_id.clone());

    TaskResponse {
        task_id: row.task_id.clone(),
        short_id: row.short_id.clone(),
        title,
        name: row.name.clone(),
        description: row.description.clone(),
        status: row.status,
        agent_type: row.agent_type.clone(),
        repository_id: row.repository_id.clone(),
        repository_name: names.repository,
        workspace_id: row.workspace_id.clone(),
        workspace_name: names.workspace,
        branch: row.branch.clone(),
        model: row.model.clone(),
        created_at: row.created_at.clone(),
        last_message_at: row.last_message_at.clone(),
        total_cost_usd: row.total_cost_usd,
        site_url: row.site_url.clone(),
        template_id: row.template_id.clone(),
        vibe: row.vibe,
        created_by: row.created_by.clone(),
        created_by_display_name: names.creator,
        visibility: row.visibility,
        scope: row.scope,
        owner_user_id: row.owner_user_id.clone(),
        parent_task_id,
        relations,
        append_system_prompt: row.append_system_prompt.clone(),
        tags,
        bindings,
        ttl_minutes: row.ttl_minutes,
        inactive_ttl_days: row.inactive_ttl_days,
        sandbox_started_at: sandbox.started_at.map(|t| t.to_rfc3339()),
        sandbox_desired_state: sandbox.desired_state,
        sandbox_stop_reason: sandbox.stop_reason,
        sandbox_desired_state_updated_at: sandbox.desired_state_updated_at.to_rfc3339(),
        // Populated per-request by callers that know the requesting user.
        last_read_at: None,
        is_following: None,
    }
}

/// Best-effort batch load of outgoing relations for many task ids, grouped
/// into a `task_id -> Vec<TaskRelationDto>` map. Failures are logged and
/// produce an empty map (callers fall back to empty `relations`).
async fn load_outgoing_relations_batch(
    state: &AppState,
    task_ids: &[String],
) -> std::collections::HashMap<String, Vec<TaskRelationDto>> {
    use std::collections::HashMap;
    if task_ids.is_empty() {
        return HashMap::new();
    }
    let mut out: HashMap<String, Vec<TaskRelationDto>> = HashMap::new();
    for tid in task_ids {
        match state.task_relations.list_outgoing(tid).await {
            Ok(rows) => {
                if !rows.is_empty() {
                    out.insert(tid.clone(), relations_to_dtos(rows));
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, task_id = %tid, "Failed to load relations for task");
            }
        }
    }
    out
}

/// Translate domain TaskRelation rows into wire DTOs.
fn relations_to_dtos(rows: Vec<crate::task::relations::TaskRelation>) -> Vec<TaskRelationDto> {
    rows.into_iter()
        .map(|r| TaskRelationDto {
            id: r.id,
            from_task_id: r.from_task_id,
            to_task_id: r.to_task_id,
            relation_type: r.relation_type,
            context: r.context,
            created_at: r.created_at.to_rfc3339(),
        })
        .collect()
}

fn task_resource(row: &Task) -> ScopedResource {
    row.scoped_resource()
}

/// Derive task title: name > description first line > agent_type > "Task"
fn derive_title(row: &Task) -> String {
    if let Some(ref name) = row.name {
        if !name.is_empty() {
            return name.clone();
        }
    }
    if let Some(ref desc) = row.description {
        if !desc.is_empty() {
            return desc.lines().next().unwrap_or("Task").to_string();
        }
    }
    row.agent_type.to_string()
}

/// Resolve display names for multiple user IDs in a single batch query.
async fn resolve_user_display_names_batch(
    state: &AppState,
    user_ids: &[String],
) -> std::collections::HashMap<String, String> {
    if user_ids.is_empty() {
        return std::collections::HashMap::new();
    }
    let uuids: Vec<uuid::Uuid> = user_ids.iter().filter_map(|id| id.parse().ok()).collect();
    match state.users.get_users_by_ids(&uuids).await {
        Ok(users) => users
            .into_iter()
            .map(|u| {
                let display_name = u.name.unwrap_or(u.login);
                (u.id.to_string(), display_name)
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "Failed to batch resolve user display names");
            std::collections::HashMap::new()
        }
    }
}

/// Resolve a single user's display name.
async fn resolve_single_user_display_name(state: &AppState, user_id: &str) -> Option<String> {
    let uid = user_id.parse::<uuid::Uuid>().ok()?;
    match state.users.get_user(&uid).await {
        Ok(Some(user)) => Some(user.name.unwrap_or(user.login)),
        _ => None,
    }
}

/// Resolve external bindings for a single task (used by enrich_task).
async fn resolve_task_bindings(state: &AppState, task_id: &str) -> Vec<TaskBindingSummary> {
    let bindings = match state.task_bindings.find_by_task_id(task_id).await {
        Ok(bindings) => bindings,
        Err(e) => {
            tracing::warn!(error = %e, task_id = %task_id, "Failed to resolve task bindings");
            return vec![];
        }
    };

    let mut summaries = Vec::new();
    for binding in &bindings {
        let trigger = if let Some(ref loop_id) = binding.loop_id {
            state
                .loops
                .find_subscription_by_id(loop_id)
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        summaries.push(crate::bindings::schemas::task_binding_summary_from_binding(
            binding,
            trigger.as_ref(),
        ));
    }
    summaries
}

/// Resolve workspace names for multiple workspace IDs in a single batch query.
async fn resolve_workspace_names_batch(
    state: &AppState,
    workspace_ids: &[String],
) -> std::collections::HashMap<String, String> {
    if workspace_ids.is_empty() {
        return std::collections::HashMap::new();
    }
    match state.workspaces.find_names_by_ids(workspace_ids).await {
        Ok(pairs) => pairs.into_iter().collect(),
        Err(e) => {
            tracing::warn!(error = %e, "Failed to batch resolve workspace names");
            std::collections::HashMap::new()
        }
    }
}

/// Resolve repository names for multiple repo IDs in a single batch query.
async fn resolve_repo_names_batch(
    state: &AppState,
    repo_ids: &[String],
) -> std::collections::HashMap<String, String> {
    if repo_ids.is_empty() {
        return std::collections::HashMap::new();
    }
    match state.repositories.find_names_by_ids(repo_ids).await {
        Ok(pairs) => pairs.into_iter().collect(),
        Err(e) => {
            tracing::warn!(error = %e, "Failed to batch resolve repository names");
            std::collections::HashMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::adapters::models::TaskBinding;
    use crate::bindings::dao::LoopRecentTaskRow;

    /// `TaskBindingDao` fake that only answers `deactivate_by_task`.
    ///
    /// It records the task ids it was asked to deactivate and replays a queued
    /// outcome, which is everything [`deactivate_task_bindings`] can observe.
    /// The remaining port methods are unreachable from this seam and panic
    /// rather than pretending to have behaviour no test asserts.
    struct FakeBindings {
        outcome: Mutex<Option<Result<u64, sqlx::Error>>>,
        deactivated: Mutex<Vec<String>>,
    }

    impl FakeBindings {
        fn returning(outcome: Result<u64, sqlx::Error>) -> Self {
            Self {
                outcome: Mutex::new(Some(outcome)),
                deactivated: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.deactivated.lock().expect("fake poisoned").clone()
        }
    }

    #[async_trait::async_trait]
    impl TaskBindingDao for FakeBindings {
        async fn deactivate_by_task(&self, task_id: &str) -> Result<u64, sqlx::Error> {
            self.deactivated
                .lock()
                .expect("fake poisoned")
                .push(task_id.to_string());
            self.outcome
                .lock()
                .expect("fake poisoned")
                .take()
                .expect("deactivate_by_task called more times than the test queued outcomes")
        }

        async fn find_by_id(&self, _id: &str) -> Result<Option<TaskBinding>, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn find_by_subject(
            &self,
            _provider: &str,
            _subject_kind: &str,
            _subject_key: &str,
        ) -> Result<Vec<TaskBinding>, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn find_all_by_subject_excluding_task(
            &self,
            _provider: &str,
            _subject_kind: &str,
            _subject_key: &str,
            _exclude_task_id: &str,
        ) -> Result<Vec<TaskBinding>, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn find_by_task_id(&self, _task_id: &str) -> Result<Vec<TaskBinding>, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn find_by_task_id_and_subject(
            &self,
            _task_id: &str,
            _provider: &str,
            _subject_kind: &str,
            _subject_key: &str,
        ) -> Result<Option<TaskBinding>, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn find_provider_event_created_for_subject(
            &self,
            _provider: &str,
            _subject_kind: &str,
            _subject_key: &str,
        ) -> Result<Option<TaskBinding>, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn find_active_by_destination_field(
            &self,
            _provider: &str,
            _subject_kind: &str,
            _field_name: &str,
            _field_value: &str,
        ) -> Result<Vec<TaskBinding>, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn upsert(&self, _row: &TaskBinding) -> Result<TaskBinding, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn delete(&self, _id: &str) -> Result<u64, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn touch_last_event(&self, _id: &str) -> Result<(), sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn touch_cursor(
            &self,
            _id: &str,
            _last_external_event_id: &str,
        ) -> Result<(), sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn update_external_status(
            &self,
            _id: &str,
            _status: Option<&str>,
            _state_kind: Option<&str>,
            _raw: &serde_json::Value,
        ) -> Result<(), sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn find_by_task_ids(
            &self,
            _task_ids: &[String],
        ) -> Result<Vec<TaskBinding>, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
        async fn find_recent_for_loop(
            &self,
            _loop_id: &str,
            _limit: i64,
        ) -> Result<Vec<LoopRecentTaskRow>, sqlx::Error> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }
    }

    /// The close/delete cleanup reaches `task_bindings` only through the
    /// injected port — no pool, no concrete repository.
    #[tokio::test]
    async fn deactivate_task_bindings_goes_through_the_injected_port() {
        let bindings = FakeBindings::returning(Ok(2));

        deactivate_task_bindings(&bindings, "task-1").await;

        assert_eq!(bindings.calls(), vec!["task-1".to_string()]);
    }

    /// Deactivation is best-effort: a repository failure is logged, not
    /// propagated, so closing or deleting a task still completes.
    #[tokio::test]
    async fn deactivate_task_bindings_swallows_a_repository_error() {
        let bindings = FakeBindings::returning(Err(sqlx::Error::PoolClosed));

        deactivate_task_bindings(&bindings, "task-2").await;

        assert_eq!(bindings.calls(), vec!["task-2".to_string()]);
    }
}
