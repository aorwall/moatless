//! Task HTTP handlers — coordinate repository calls + enrichment.

#[cfg(test)]
use super::bindings::deactivate_task_bindings;
use super::closer::TaskCloser;
use super::creation::TaskCreationService;
use super::error::TaskError;
use super::fork::TaskForkService;
use super::models::{TaskFilter, TaskPatch};
use super::operations::TaskCommands;
use super::presenter::{ResolvedNames, TaskPresenter, build_task_response};
use super::schemas::*;
use super::service;
use super::switch::{SwitchCredentials, SwitchOutcome, TaskSwitchService};
use crate::credential_sync::CredentialSync;
use crate::message::operations::Messages;
use crate::message::send::MessageSendService;
use crate::tag::operations::Tags;
use crate::task::bindings::TaskBindings;
use crate::task::models::SandboxIntentView;
#[cfg(test)]
use crate::task::models::TaskStatus;
use crate::task_turn::operations::TaskTurns;
use axum::Json;
use axum::extract::{Path, Query, State};
use moatless_api_schemas::bindings::TaskBindingSummary;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_core::http_auth::AuthUser;
use moatless_ports::identity::{HarnessCredentials, SecretCatalog, UserResolver};
use moatless_ports::sandbox::PreflightPort;
use moatless_ports::sandbox::SandboxLifecycle;
#[cfg(test)]
use moatless_types::sandbox::ArchiveRetention;
use moatless_types::task::TaskSubType;
use moatless_workspace_ports::{RepositoryReader, WorkspaceResolver};
use moatless_workspaces::workspace::git_sync::GitSyncService;
use std::sync::{Arc, Mutex};

/// Create a new task.
#[utoipa::path(
    post,
    path = "/api/v1/tasks",
    tag = "tasks",
    request_body = CreateTaskRequest,
    responses(
        (status = 200, description = "Task created", body = CreateTaskResponse),
        (status = 400, description = "Invalid request or unusable Codex credential", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "GitHub not connected, or no access to the repository", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Repository not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "eight ports, all forwarded: this handler is the outermost caller of a two-stage service pipeline — five to service::normalize, two to service::create_task, one to the presenter closure. Unlike the service-layer create_task this was split out of, there is no caller above a handler to hoist them to, so the width cannot move anywhere it would be narrower."
)]
pub async fn create_task(
    State(workspaces): State<Arc<dyn WorkspaceResolver>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    State(tasks): State<Arc<TaskCommands>>,
    State(users): State<Arc<dyn UserResolver>>,
    State(credential_sync): State<Arc<CredentialSync>>,
    State(config): State<Arc<Config>>,
    State(task_creation): State<Arc<TaskCreationService>>,
    State(message_send): State<Arc<MessageSendService>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    auth: AuthUser,
    Json(req): Json<CreateTaskRequest>,
) -> Result<Json<CreateTaskResponse>, TaskError> {
    // enrich_task is DB-heavy (~6 queries); enrich exactly once. The event
    // closure stashes the enriched response here and the HTTP response reuses
    // it, avoiding both a second round of queries and an event/response
    // divergence window.
    let user_id = auth.user_id.to_string();
    let enriched: Arc<Mutex<Option<TaskResponse>>> = Arc::new(Mutex::new(None));

    let presenter_ev = task_presenter.clone();
    let uid_ev = user_id.clone();
    let enriched_ev = enriched.clone();
    let new_task = service::normalize(
        service::CreateTaskPorts {
            workspaces: workspaces.as_ref(),
            repositories: repositories.as_ref(),
            tasks: tasks.as_ref(),
            users: users.as_ref(),
            credential_sync: &credential_sync,
            config: &config,
        },
        &auth.scope_context(),
        &user_id,
        service::CreateTaskIntent::new(req),
    )
    .await?;
    let outcome = service::create_task(
        &task_creation,
        message_send,
        new_task,
        move |task| async move {
            match presenter_ev.enrich_task(&task, &uid_ev).await {
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
        None => task_presenter.enrich_task(&outcome.task, &user_id).await?,
    };
    Ok(Json(CreateTaskResponse {
        task_id: outcome.task.task_id.clone(),
        status: outcome.task.status,
        task: task_response,
    }))
}

/// Fork a task: branch a new Task off this one at one of its completed turns.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/fork",
    tag = "tasks",
    request_body = ForkTaskRequest,
    params(
        ("task_id" = String, Path, description = "Task to branch off")
    ),
    responses(
        (status = 200, description = "Fork created", body = CreateTaskResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "No access to the source task's repository", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task or turn not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "The cut reaches a turn still in flight, or the turn has no agent turn id", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "the same two-stage create pipeline `create_task` forwards to, plus the fork service that resolves the cut and copies the conversation ahead of it, and the switch service a harness choice resolves through. As there, no caller above a handler exists to hoist the ports to."
)]
pub async fn fork_task(
    State(workspaces): State<Arc<dyn WorkspaceResolver>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    State(tasks): State<Arc<TaskCommands>>,
    State(users): State<Arc<dyn UserResolver>>,
    State(credential_sync): State<Arc<CredentialSync>>,
    State(config): State<Arc<Config>>,
    State(task_creation): State<Arc<TaskCreationService>>,
    State(task_fork): State<Arc<TaskForkService>>,
    State(task_switch): State<Arc<TaskSwitchService>>,
    State(message_send): State<Arc<MessageSendService>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<ForkTaskRequest>,
) -> Result<Json<CreateTaskResponse>, TaskError> {
    let user_id = auth.user_id.to_string();
    let enriched: Arc<Mutex<Option<TaskResponse>>> = Arc::new(Mutex::new(None));
    let presenter_ev = task_presenter.clone();
    let uid_ev = user_id.clone();
    let enriched_ev = enriched.clone();
    let outcome = task_fork.fork(super::fork::ForkCreationPorts {
        normalize: service::CreateTaskPorts {
            workspaces: workspaces.as_ref(), repositories: repositories.as_ref(),
            tasks: tasks.as_ref(), users: users.as_ref(),
            credential_sync: &credential_sync, config: &config,
        },
        task_creation: &task_creation,
        message_send,
        switch: &task_switch,
    }, &auth.scope_context(), super::fork::ForkRequest::minted(&task_id, req), move |task| async move {
            match presenter_ev.enrich_task(&task, &uid_ev).await {
                Ok(resp) => {
                    let payload = serde_json::json!({ "task": resp });
                    if let Ok(mut cell) = enriched_ev.lock() {
                        *cell = Some(resp);
                    }
                    payload
                }
                Err(e) => {
                    tracing::warn!(task_id = %task.task_id, error = %e, "Failed to enrich forked task for event");
                    serde_json::json!({ "task": { "taskId": task.task_id } })
                }
            }
        }).await?;
    let stashed = enriched.lock().ok().and_then(|mut cell| cell.take());
    let task_response = match stashed {
        Some(resp) => resp,
        None => task_presenter.enrich_task(&outcome.task, &user_id).await?,
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
    State(tasks): State<Arc<TaskCommands>>,
    State(tags): State<Arc<Tags>>,
    State(task_bindings): State<Arc<dyn TaskBindings>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
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

    let (rows, total) = tasks.list_tasks(&filter).await?;

    let task_ids: Vec<String> = rows.iter().map(|r| r.task_id.clone()).collect();
    let all_tags = tags.get_tags_for_tasks(&task_ids).await?;

    // Durable Sandbox state comes from the `sandboxes` table; absent rows fall
    // back to the lazy converged-Stopped default in the loop below.
    let sandbox_intents = tasks
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
    let user_names = task_presenter
        .resolve_user_display_names_batch(&creator_ids)
        .await;

    // Batch resolve repository names (single query instead of N+1)
    let repo_ids: Vec<String> = rows
        .iter()
        .map(|r| r.repository_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let repo_names = task_presenter.resolve_repo_names_batch(&repo_ids).await;

    let workspace_ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r.workspace_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let workspace_names = task_presenter
        .resolve_workspace_names_batch(&workspace_ids)
        .await;

    // Batch resolve outgoing relations for tasks (single query instead of N+1).
    let all_relations = task_presenter
        .load_outgoing_relations_batch(&task_ids)
        .await;

    // Batch resolve external bindings for tasks (single query instead of N+1)
    let all_bindings = task_bindings
        .list_summaries(&task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve task bindings");
            Default::default()
        });

    // Batch resolve the requesting user's read markers (single query). Drives
    // the unread indicator; best-effort — empty on error.
    let read_receipts = tasks
        .get_read_receipts(&auth.user_id.to_string(), &task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve read receipts");
            Default::default()
        });

    // Batch resolve which of these tasks the requesting user follows (single
    // query). Drives the follow indicator; best-effort — empty on error.
    let followed = tasks
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

            let bindings: Vec<TaskBindingSummary> =
                all_bindings.get(&row.task_id).cloned().unwrap_or_default();

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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_task(
    State(tasks): State<Arc<TaskCommands>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<TaskResponse>, TaskError> {
    let row = tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = task_presenter
        .enrich_task(&row, &auth.user_id.to_string())
        .await?;
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_task_statistics(
    State(tasks): State<Arc<TaskCommands>>,
    State(messages_dao): State<Arc<Messages>>,
    State(task_turns): State<Arc<TaskTurns>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<TaskStatisticsResponse>, TaskError> {
    let row = tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let (messages, _, activities) = messages_dao.get_all(&task_id).await?;
    let usage = messages_dao.response_usage(&task_id).await?;
    let turn_metrics = task_turns
        .list_turn_metrics(&task_id)
        .await
        .unwrap_or_default();
    Ok(Json(super::statistics::build_task_statistics(
        &row,
        &messages,
        &usage,
        &activities,
        &turn_metrics,
    )))
}

/// Update a task's name, description, model or Agent profile.
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
        (status = 400, description = "Unknown agentProfile, missing credential, or a model the agentProfile does not offer", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "The Task or its Sandbox cannot run the agentProfile", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[allow(clippy::too_many_arguments)]
pub async fn update_task_metadata(
    State(tasks): State<Arc<TaskCommands>>,
    State(task_switch): State<Arc<TaskSwitchService>>,
    State(secrets): State<Arc<dyn SecretCatalog>>,
    State(provider_tokens): State<Arc<dyn HarnessCredentials>>,
    State(credential_sync): State<Arc<CredentialSync>>,
    State(events): State<Option<Arc<dyn EventPublisher>>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskMetadataRequest>,
) -> Result<Json<TaskResponse>, TaskError> {
    if let Some(ref name) = req.name {
        service::validate_task_name(name)?;
    }

    let scope_ctx = auth.scope_context();
    let switched = match req.agent_profile.as_deref() {
        Some(profile) => {
            let profile = profile.parse().map_err(TaskError::BadRequest)?;
            task_switch
                .switch_profile(
                    &scope_ctx,
                    &task_id,
                    profile,
                    req.model.as_deref(),
                    SwitchCredentials {
                        secrets: secrets.as_ref(),
                        provider_tokens: provider_tokens.as_ref(),
                        credential_sync: credential_sync.as_ref(),
                    },
                )
                .await?
                != SwitchOutcome::Unchanged
        }
        None => false,
    };
    if let Some(model) = req.model.as_ref().filter(|_| !switched) {
        // Read the agent profile before the write: a model belonging to another
        // agent is refused at creation, and a patch must not slip one past it.
        let row = tasks
            .find_by_id(&task_id, &scope_ctx)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
        crate::agent::catalog::validate_agent_model(row.agent_profile, Some(model))
            .map_err(TaskError::BadRequest)?;
    }
    let updated = tasks
        .update_task(
            &task_id,
            &TaskPatch {
                name: req.name.clone(),
                description: req.description.clone(),
                // A switch wrote the model with the profile.
                model: if switched { None } else { req.model.clone() },
                ..Default::default()
            },
            &scope_ctx,
        )
        .await?;
    if !updated {
        return Err(TaskError::NotFound(task_id.clone()));
    }
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task metadata updated");

    let row = tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    if let Some(ref events) = events {
        events
            .publish_task_event(
                &task_id,
                TaskSubType::MetadataUpdated,
                Some(row.agent_profile.as_str()),
                serde_json::json!({
                    "name": req.name,
                    "description": req.description,
                    "model": row.model,
                    "agentProfile": row.agent_profile.as_str(),
                }),
            )
            .await;
    }

    let resp = task_presenter
        .enrich_task(&row, &auth.user_id.to_string())
        .await?;
    Ok(Json(resp))
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_task(
    State(sandbox): State<Arc<dyn SandboxLifecycle>>,
    State(task_bindings): State<Arc<dyn TaskBindings>>,
    State(tasks): State<Arc<TaskCommands>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    super::removal::delete_task(
        sandbox.as_ref(),
        task_bindings.as_ref(),
        tasks.as_ref(),
        &auth.scope_context(),
        task_id,
    )
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Mark a task's conversation as read for the requesting user, up to now.
/// Clears the unread indicator; safe to call repeatedly (the stored marker only
/// ever advances).
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/read",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Task marked as read"),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn mark_task_read(
    State(tasks): State<Arc<TaskCommands>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    tasks
        .mark_task_read(&auth.user_id.to_string(), &task_id, chrono::Utc::now())
        .await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Follow a task so it appears in the requesting user's personal task list. Any
/// task the user can see may be followed. Idempotent.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/follow",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task followed", body = FollowTaskResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn follow_task(
    State(tasks): State<Arc<TaskCommands>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<FollowTaskResponse>, TaskError> {
    tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    tasks
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn unfollow_task(
    State(tasks): State<Arc<TaskCommands>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<FollowTaskResponse>, TaskError> {
    tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    tasks
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
        (status = 400, description = "Child-close choice required", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn close_task(
    State(task_closer): State<Arc<TaskCloser>>,
    State(tasks): State<Arc<TaskCommands>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    req: Option<Json<CloseTaskRequest>>,
) -> Result<Json<TaskResponse>, TaskError> {
    task_closer
        .close_requested(
            &auth.scope_context(),
            task_id.clone(),
            req.map(|Json(req)| req).unwrap_or_default(),
        )
        .await?;
    let updated = tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = task_presenter
        .enrich_task(&updated, &auth.user_id.to_string())
        .await?;
    Ok(Json(resp))
}

/// Reopen a closed task, flipping `Closed` to `Open`. No sandbox is provisioned
/// here — the next `start` restores retained S3 state if
/// `SANDBOX_RETAIN_STORAGE_ON_CLOSE` was set; bindings stay inactive.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/reopen",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task reopened", body = TaskResponse),
        (status = 400, description = "Task is not closed", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn reopen_task(
    State(tasks): State<Arc<TaskCommands>>,
    State(events): State<Option<Arc<dyn EventPublisher>>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<TaskResponse>, TaskError> {
    reopen_task_for(
        tasks.as_ref(),
        events.as_deref(),
        &auth.scope_context(),
        &task_id,
    )
    .await?;

    let updated = tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = task_presenter
        .enrich_task(&updated, &auth.user_id.to_string())
        .await?;
    Ok(Json(resp))
}

/// Reopen one closed task on `scope_ctx`'s behalf. The status check is what
/// makes this safe as an undo — a task that was never closed is refused rather
/// than quietly rewritten.
pub use super::service::reopen_task_for;

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
        (status = 400, description = "Invalid task state", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found, or the caller has no control access to it", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "six ports: starting a task touches the task row, its repository, preflight, events, the sandbox and git-sync; each is read here, none forwarded"
)]
pub async fn start_task(
    State(tasks): State<Arc<TaskCommands>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    State(preflight): State<Arc<dyn PreflightPort>>,
    State(events): State<Option<Arc<dyn EventPublisher>>>,
    State(sandbox): State<Arc<dyn SandboxLifecycle>>,
    State(git_sync): State<Arc<GitSyncService>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    super::start::start_task(
        super::start::StartTaskPorts {
            tasks: tasks.as_ref(),
            repositories: repositories.as_ref(),
            preflight: preflight.as_ref(),
            events: events.as_deref(),
            sandbox: sandbox.as_ref(),
            git_sync: git_sync.as_ref(),
        },
        &auth.scope_context(),
        task_id,
    )
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn stop_task(
    State(tasks): State<Arc<TaskCommands>>,
    State(sandbox): State<Arc<dyn SandboxLifecycle>>,
    State(events): State<Option<Arc<dyn EventPublisher>>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    service::stop_task(
        tasks.as_ref(),
        sandbox.as_ref(),
        events.as_deref(),
        &auth.scope_context(),
        &task_id,
    )
    .await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Cancel a task's run and every created-by descendant's: dispose their
/// pending Messages, then stop each open one's agent.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/cancel",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task cancelled", body = CancelTaskResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn cancel_task(
    State(task_closer): State<Arc<TaskCloser>>,
    State(message_send): State<Arc<MessageSendService>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<CancelTaskResponse>, TaskError> {
    let outcome = task_closer
        .cancel_requested(&auth.scope_context(), &task_id, &message_send)
        .await?;
    Ok(Json(CancelTaskResponse {
        stopped_task_ids: outcome.stopped_task_ids,
        disposed_messages: outcome.disposed_messages,
    }))
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn set_task_tags(
    State(tasks): State<Arc<TaskCommands>>,
    State(tags_dao): State<Arc<Tags>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SetTagsRequest>,
) -> Result<Json<Vec<TagResponse>>, TaskError> {
    let scope_ctx = auth.scope_context();
    let existing = tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    if !scope_ctx.can_write(&existing.scoped_resource()) {
        return Err(TaskError::NotFound(task_id.clone())); // existence hiding
    }

    let tags = tags_dao.find_or_create_tags(&req.tags).await?;
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();
    tags_dao.set_task_tags(&task_id, &tag_ids).await?;
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
        (status = 400, description = "Invalid visibility value", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn update_visibility(
    State(tasks): State<Arc<TaskCommands>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateVisibilityRequest>,
) -> Result<Json<TaskResponse>, TaskError> {
    let scope_ctx = auth.scope_context();
    let updated = tasks
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

    let row = tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = task_presenter
        .enrich_task(&row, &auth.user_id.to_string())
        .await?;
    Ok(Json(resp))
}

/// Update a task's TTLs. `ttl_minutes` is idle TTL (`0` disables the reaper);
/// `inactive_ttl_days` is inactivity TTL (`0` disables auto-close). Both
/// optional; at least one required.
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
        (status = 400, description = "Invalid TTL value", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn update_ttl(
    State(tasks): State<Arc<TaskCommands>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskTtlRequest>,
) -> Result<Json<TaskResponse>, TaskError> {
    let row = service::update_ttl(tasks.as_ref(), &auth.scope_context(), task_id, req).await?;
    let resp = task_presenter
        .enrich_task(&row, &auth.user_id.to_string())
        .await?;
    Ok(Json(resp))
}

// =========================================================================
// Helpers
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    use crate::task::dao::MockTaskDao;
    use crate::task::models::{SandboxDesiredState, SandboxStopReason};
    use moatless_types::store::StoreError;
    use uuid::Uuid;

    struct RecordingSandbox {
        log: Arc<Mutex<Vec<&'static str>>>,
        fail_removal: bool,
    }

    #[async_trait::async_trait]
    impl SandboxLifecycle for RecordingSandbox {
        async fn start(
            &self,
            _task_id: &str,
        ) -> Result<(), moatless_ports::sandbox::SandboxOpError> {
            unimplemented!("not reachable from delete_task ordering test")
        }

        async fn stop(
            &self,
            _task_id: &str,
            _reason: SandboxStopReason,
        ) -> Result<(), moatless_ports::sandbox::SandboxOpError> {
            unimplemented!("not reachable from delete_task ordering test")
        }

        async fn stop_agent(&self, _task_id: &str, _reason: SandboxStopReason) {
            unimplemented!("not reachable from delete_task ordering test")
        }

        async fn remove(
            &self,
            _task_id: &str,
            _retention: ArchiveRetention,
        ) -> Result<(), moatless_ports::sandbox::SandboxOpError> {
            self.log.lock().expect("log").push("remove");
            if self.fail_removal {
                return Err(moatless_ports::sandbox::SandboxOpError::Internal(
                    "remove failed".to_string(),
                ));
            }
            Ok(())
        }

        async fn archive_storage(
            &self,
            _task_id: &str,
        ) -> Result<(), moatless_ports::sandbox::SandboxOpError> {
            unimplemented!("not reachable from delete_task ordering test")
        }

        async fn stopped_with_storage(
            &self,
            _task_ids: &[String],
        ) -> Result<
            std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
            moatless_ports::sandbox::SandboxOpError,
        > {
            unimplemented!("not reachable from delete_task ordering test")
        }

        async fn desired_state(
            &self,
            _task_id: &str,
        ) -> Result<SandboxDesiredState, moatless_ports::sandbox::SandboxOpError> {
            unimplemented!("not reachable from delete_task ordering test")
        }
    }

    /// `TaskBindings` fake answering only `deactivate_for_task`: records
    /// requested task ids and replays a queued outcome — everything
    /// [`deactivate_task_bindings`] can observe. Other methods panic.
    struct FakeBindings {
        outcome: Mutex<Option<Result<u64, StoreError>>>,
        deactivated: Mutex<Vec<String>>,
    }

    impl FakeBindings {
        fn returning(outcome: Result<u64, StoreError>) -> Self {
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
    impl TaskBindings for FakeBindings {
        async fn deactivate_for_task(&self, task_id: &str) -> Result<u64, StoreError> {
            self.deactivated
                .lock()
                .expect("fake poisoned")
                .push(task_id.to_string());
            self.outcome
                .lock()
                .expect("fake poisoned")
                .take()
                .expect("deactivate_for_task called more times than the test queued outcomes")
        }

        async fn summaries(&self, _task_id: &str) -> Result<Vec<TaskBindingSummary>, StoreError> {
            unimplemented!("not reachable from deactivate_task_bindings")
        }

        async fn list_summaries(
            &self,
            _task_ids: &[String],
        ) -> Result<std::collections::HashMap<String, Vec<TaskBindingSummary>>, StoreError>
        {
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
        let bindings = FakeBindings::returning(Err(StoreError::new("pool closed")));

        deactivate_task_bindings(&bindings, "task-2").await;

        assert_eq!(bindings.calls(), vec!["task-2".to_string()]);
    }

    #[tokio::test]
    async fn delete_closes_task_before_lifecycle_remove() {
        let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = MockTaskDao::new();
        let update_log = Arc::clone(&log);
        tasks
            .expect_update_task()
            .withf(|task_id, patch, _ctx| {
                task_id == "task-1" && patch.status == Some(TaskStatus::Closed)
            })
            .once()
            .returning(move |_, _, _| {
                update_log.lock().expect("log").push("close");
                Ok(true)
            });
        let delete_log = Arc::clone(&log);
        tasks
            .expect_delete_task()
            .withf(|task_id, _ctx| task_id == "task-1")
            .once()
            .returning(move |_, _| {
                delete_log.lock().expect("log").push("delete");
                Ok(true)
            });

        let status = delete_task(
            State(Arc::new(RecordingSandbox {
                log: Arc::clone(&log),
                fail_removal: false,
            }) as Arc<dyn SandboxLifecycle>),
            State(Arc::new(FakeBindings::returning(Ok(0))) as Arc<dyn TaskBindings>),
            State(Arc::new(TaskCommands::new(Arc::new(tasks)))),
            AuthUser {
                user_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001")
                    .expect("valid uuid"),
                login: "user-1".to_string(),
                role: None,
                provider: None,
                group_id: None,
                task_id: None,
                is_agent_context: false,
            },
            Path("task-1".to_string()),
        )
        .await
        .expect("delete succeeds");

        assert_eq!(status, axum::http::StatusCode::NO_CONTENT);
        assert_eq!(*log.lock().expect("log"), vec!["close", "remove", "delete"]);
    }

    #[tokio::test]
    async fn failed_removal_retains_the_closed_task_row() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = MockTaskDao::new();
        let updated = log.clone();
        tasks
            .expect_update_task()
            .once()
            .withf(|_, patch, _| patch.status == Some(TaskStatus::Closed))
            .returning(move |_, _, _| {
                updated.lock().unwrap().push("close");
                Ok(true)
            });
        let sandbox = RecordingSandbox {
            log: log.clone(),
            fail_removal: true,
        };
        let bindings = FakeBindings::returning(Ok(1));
        let ctx = moatless_types::access_control::ScopeContext::user("owner");
        let tasks = TaskCommands::new(Arc::new(tasks));
        let result = super::super::removal::delete_task(
            &sandbox,
            &bindings,
            &tasks,
            &ctx,
            "task-1".to_string(),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(*log.lock().unwrap(), vec!["close", "remove"]);
        assert_eq!(*bindings.deactivated.lock().unwrap(), vec!["task-1"]);
    }
}
