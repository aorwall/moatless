//! Task HTTP handlers — coordinate repository calls + enrichment.

use axum::extract::{Path, Query, State};
use axum::Json;

use moatless_events::{SandboxEventPayload, SandboxSubType, TaskSubType};

use super::dao::{TaskFilter, TaskPatch};
use super::models::Task;
use crate::extractors::AuthUser;
use crate::shared_models::{AgentMode, AgentType};
use crate::state::AppState;
use crate::task::models::{TaskStatus, Visibility};

use crate::repository::models::{RepositoryKind, RepositoryProvider};

use super::error::TaskError;
use super::schemas::*;

/// Create a new task.
#[utoipa::path(
    post,
    path = "/api/v1/tasks",
    tag = "tasks",
    request_body = CreateTaskRequest,
    responses(
        (status = 200, description = "Task created", body = CreateTaskResponse),
        (status = 403, description = "No access to GitHub repository", body = crate::error::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn create_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateTaskRequest>,
) -> Result<Json<CreateTaskResponse>, TaskError> {
    // Validate repository exists
    let repo = state.repositories.find_repository_by_id(&req.repository_id)
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(req.repository_id.clone()))?;

    // Verify GitHub repository access for non-template repos
    if repo.provider == Some(RepositoryProvider::Github) && repo.kind != RepositoryKind::Template {
        if let Some(ref remote_url) = repo.remote_url {
            if let Some((owner, repo_name)) =
                crate::repository::handlers::parse_github_owner_repo(remote_url)
            {
                // Look up user's GitHub token, falling back to team token
                let token = state
                    .provider_tokens
                    .find_provider_token(&auth.user_id, "github")
                    .await
                    .map_err(|e| {
                        TaskError::Internal(format!("Failed to look up GitHub token: {e}"))
                    })?;

                let access_token = if let Some(t) = token {
                    t.access_token
                } else {
                    let team_token = if let Some(ref team_id) = repo.team_id {
                        state
                            .provider_tokens
                            .find_team_token(team_id, "github")
                            .await
                            .map_err(|e| {
                                TaskError::Internal(format!(
                                    "Failed to look up team GitHub token: {e}"
                                ))
                            })?
                    } else {
                        None
                    };
                    if let Some(t) = team_token {
                        t.access_token
                    } else {
                        return Err(TaskError::Forbidden(
                            "GitHub account not connected. Please connect your GitHub account to create tasks for this repository.".to_string(),
                        ));
                    }
                };

                crate::provider_settings::github_client::check_github_repo_access(
                    &access_token,
                    &owner,
                    &repo_name,
                )
                .await
                .map_err(TaskError::Forbidden)?;
            }
        }
    }

    // Validate name if provided
    if let Some(ref name) = req.name {
        validate_task_name(name)?;
    }

    let is_template = repo.kind == RepositoryKind::Template;

    let task_id = uuid::Uuid::new_v4().to_string();
    let short_id = generate_unique_short_id(&state).await?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let agent_type = req.agent_type.unwrap_or_else(|| AgentType(AgentType::CLAUDE_CODE.into()));

    let row = Task {
        task_id: task_id.clone(),
        short_id: Some(short_id),
        created_at: now.clone(),
        last_message_at: now,
        agent_type,
        repository_id: req.repository_id.clone(),
        status: TaskStatus::Open,
        total_cost_usd: 0.0,
        site_url: None,
        template_id: if is_template { Some(req.repository_id.clone()) } else { None },
        vibe: if is_template { Some(true) } else { None },
        name: req.name.clone(),
        description: req.description.clone(),
        site_id: None,
        site_provider: None,
        branch: req.branch.clone().or(repo.default_branch.clone()),
        created_by: Some(auth.user_id.clone()),
        visibility: Visibility::Private,
        scope: repo.scope,
        team_id: repo.team_id.clone(),
        created_at_db: None,
        updated_at: None,
        parent_task_id: req.parent_task_id.clone(),
    };

    insert_and_provision_task(
        &state,
        &auth,
        &row,
        req.message.as_deref(),
        req.model.as_deref(),
        req.agent_mode.as_ref(),
        &req.file_ids,
        &req.contexts,
    )
    .await
}

/// Shared logic for creating a task: insert, publish event, provision sandbox, send initial message.
#[allow(clippy::too_many_arguments)]
async fn insert_and_provision_task(
    state: &AppState,
    auth: &AuthUser,
    row: &Task,
    message: Option<&str>,
    model: Option<&str>,
    agent_mode: Option<&AgentMode>,
    file_ids: &[String],
    contexts: &[MessageContextInput],
) -> Result<Json<CreateTaskResponse>, TaskError> {
    let task_id = &row.task_id;

    state.tasks.insert_task(row).await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, repository_id = %row.repository_id, "Task created");

    // Emit task event
    if let Some(ref events) = state.events {
        events
            .publish_task_event(task_id, TaskSubType::Open, Some(row.agent_type.as_str()), serde_json::json!({}))
            .await;
    }

    // Always provision sandbox
    match crate::sandbox::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
        task_id,
    )
    .await
    {
        Ok(request) => {
            match state.sandbox.provision(task_id, &request).await {
                Ok(_response) => {
                    tracing::info!(task_id = %task_id, "Sandbox provisioning started (initializing)");
                }
                Err(e) => {
                    tracing::warn!(task_id = %task_id, error = %e, "Failed to provision sandbox");
                    if let Some(ref events) = state.events {
                        events
                            .publish_sandbox_event_with_payload(
                                task_id,
                                SandboxSubType::Error,
                                &SandboxEventPayload {
                                    message: format!("Failed to provision sandbox: {e}"),
                                    error: Some(e.to_string()),
                                    ..Default::default()
                                },
                            )
                            .await;
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to build provision request");
            if let Some(ref events) = state.events {
                events
                    .publish_sandbox_event_with_payload(
                        task_id,
                        SandboxSubType::Error,
                        &SandboxEventPayload {
                            message: format!("Failed to build provision request: {e}"),
                            error: Some(e.to_string()),
                            ..Default::default()
                        },
                    )
                    .await;
            }
        }
    }

    // If a message was provided, send it to the agent
    if let Some(msg) = message {
        if let Err(e) = crate::message::send::send_message_flow(
            state, task_id, msg, model, agent_mode, file_ids, contexts, true, None,
        )
        .await
        {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to send initial message");
        }
    }

    let task_response = enrich_task(row, state, &auth.user_id).await?;

    Ok(Json(CreateTaskResponse {
        task_id: row.task_id.clone(),
        status: row.status,
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

    let scope_ctx = auth.scope_context();
    let filter = TaskFilter {
        user_id: scope_ctx.user_id.clone(),
        team_ids: scope_ctx.team_ids.clone(),
        is_admin: scope_ctx.is_admin,
        limit,
        offset,
        status: query.status,
        repository_id: query.repository_id.clone(),
        created_by: query.created_by.clone(),
        tag: query.tag.clone(),
        parent_task_id: query.parent_task_id.clone(),
    };

    let (rows, total) = state.tasks.list_tasks(&filter).await?;

    let task_ids: Vec<String> = rows.iter().map(|r| r.task_id.clone()).collect();
    let all_tags = state.tags.get_tags_for_tasks(&task_ids).await?;

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

            build_task_response(row, tags, display_name, repo_name)
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
    let row = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&row, &state, &auth.user_id).await?;
    Ok(Json(resp))
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
        validate_task_name(name)?;
    }

    let _existing = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    state.tasks.update_task(
        &task_id,
        &TaskPatch {
            name: req.name,
            description: req.description,
            ..Default::default()
        },
    )
    .await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task metadata updated");

    let row = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&row, &state, &auth.user_id).await?;
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
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    let _existing = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    // Cleanup workspace (best-effort)
    if let Err(e) = state.sandbox.cleanup(&task_id).await {
        tracing::warn!(task_id = %task_id, error = %e, "Failed to cleanup workspace on delete");
    }

    state.tasks.delete_task(&task_id).await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task deleted");

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Close a task and stop its sandbox.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/close",
    tag = "tasks",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task closed", body = TaskResponse),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn close_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<TaskResponse>, TaskError> {
    let row = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    // Cleanup sandbox (best-effort)
    if let Err(e) = state.sandbox.cleanup(&task_id).await {
        tracing::warn!(task_id = %task_id, error = %e, "Failed to cleanup sandbox on close");
    }
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task closed");

    state.tasks.update_task(
        &task_id,
        &TaskPatch {
            status: Some(TaskStatus::Closed),
            ..Default::default()
        },
    )
    .await?;

    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                &task_id,
                TaskSubType::Closed,
                Some(row.agent_type.as_str()),
                serde_json::json!({}),
            )
            .await;
    }

    let updated = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&updated, &state, &auth.user_id).await?;
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
    let row = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    if row.status != TaskStatus::Open && row.status != TaskStatus::Error {
        return Err(TaskError::BadRequest(format!(
            "Cannot start task in '{}' status",
            row.status
        )));
    }

    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Starting task");

    let request = crate::sandbox::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
        &task_id,
    )
    .await
    .map_err(|e| TaskError::Internal(e.to_string()))?;

    state.sandbox.provision(&task_id, &request).await
        .map_err(|e| TaskError::Internal(e.to_string()))?;

    tracing::info!(task_id = %task_id, "Sandbox provision initiated (initializing)");
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
        (status = 404, description = "Task not found", body = crate::error::ErrorBody)
    )
)]
pub async fn stop_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    let _task = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    // Stop the agent process via gRPC (keeps sandbox pod running)
    match state.sandbox.sandbox_client(&task_id).await {
        Ok(client) => {
            if let Err(e) = client.stop_agent().await {
                tracing::warn!(task_id = %task_id, error = %e, "Failed to stop agent via gRPC");
            }
        }
        Err(e) => {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to get sandbox client for agent stop");
        }
    }

    // Publish agent.stopped + taskTurn.stopped events
    // (In Docker mode the container is killed before the sandbox can publish these)
    if let Some(ref events) = state.events {
        events
            .publish_agent_event(&task_id, moatless_events::AgentSubType::Stopped, None, serde_json::json!({}))
            .await;
        events
            .publish_task_turn_event(&task_id, moatless_events::TaskTurnSubType::Stopped, None, None, serde_json::json!({}))
            .await;
    }

    // Update task status
    state.tasks.update_task(
        &task_id,
        &TaskPatch {
            status: Some(TaskStatus::Open),
            ..Default::default()
        },
    )
    .await?;

    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Agent stopped");

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
    let _existing = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

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
    let _existing = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    state.tasks.update_task(
        &task_id,
        &TaskPatch {
            visibility: Some(req.visibility),
            ..Default::default()
        },
    )
    .await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, visibility = %req.visibility, "Task visibility updated");

    let row = state.tasks.find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&row, &state, &auth.user_id).await?;
    Ok(Json(resp))
}

// =========================================================================
// Helpers
// =========================================================================

/// Enrich a single task row into a full response with tags, repo name, display name, sandbox snapshot.
async fn enrich_task(
    row: &Task,
    state: &AppState,
    _user_id: &str,
) -> Result<TaskResponse, TaskError> {
    let tags = state.tags.get_tags_for_task(&row.task_id).await?;
    let tag_responses: Vec<TagResponse> = tags
        .iter()
        .map(|t| TagResponse {
            tag_id: t.tag_id.clone(),
            name: t.name.clone(),
        })
        .collect();

    let repo_name = state.repositories.find_repository_by_id(&row.repository_id)
        .await?
        .map(|r| r.name);

    let display_name = if let Some(ref uid) = row.created_by {
        resolve_single_user_display_name(state, uid).await
    } else {
        None
    };

    Ok(build_task_response(row, tag_responses, display_name, repo_name))
}

/// Build a TaskResponse from a row + pre-resolved tags/names.
fn build_task_response(
    row: &Task,
    tags: Vec<TagResponse>,
    display_name: Option<String>,
    repo_name: Option<String>,
) -> TaskResponse {
    let title = derive_title(row);

    TaskResponse {
        task_id: row.task_id.clone(),
        short_id: row.short_id.clone(),
        title,
        name: row.name.clone(),
        description: row.description.clone(),
        status: row.status,
        agent_type: row.agent_type.clone(),
        repository_id: row.repository_id.clone(),
        repository_name: repo_name,
        branch: row.branch.clone(),
        created_at: row.created_at.clone(),
        last_message_at: row.last_message_at.clone(),
        total_cost_usd: row.total_cost_usd,
        site_url: row.site_url.clone(),
        template_id: row.template_id.clone(),
        vibe: row.vibe,
        created_by: row.created_by.clone(),
        created_by_display_name: display_name,
        visibility: row.visibility,
        scope: row.scope,
        team_id: row.team_id.clone(),
        parent_task_id: row.parent_task_id.clone(),
        tags,
        environment_status: None,
        agent_state: None,
        is_running: None,
    }
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

/// Validate task name (URL-friendly: alphanumeric + dashes).
fn validate_task_name(name: &str) -> Result<(), TaskError> {
    if name.is_empty() {
        return Err(TaskError::BadRequest("Task name cannot be empty".to_string()));
    }
    if name.len() > 200 {
        return Err(TaskError::BadRequest("Task name too long (max 200)".to_string()));
    }
    Ok(())
}

/// Generate a unique short ID for a task.
async fn generate_unique_short_id(state: &AppState) -> Result<String, TaskError> {
    for _ in 0..10 {
        let id = nanoid::nanoid!(8, &nanoid::alphabet::SAFE);
        if !state.tasks.exists_by_short_id(&id).await? {
            return Ok(id);
        }
    }
    Err(TaskError::Internal("Failed to generate unique short ID".to_string()))
}

/// Resolve display names for multiple user IDs in a single batch query.
async fn resolve_user_display_names_batch(
    state: &AppState,
    user_ids: &[String],
) -> std::collections::HashMap<String, String> {
    if user_ids.is_empty() {
        return std::collections::HashMap::new();
    }
    match state.users.get_users_by_ids(user_ids).await {
        Ok(users) => users
            .into_iter()
            .map(|u| {
                let display_name = u.name.unwrap_or(u.login);
                (u.id, display_name)
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
    match state.users.get_user(user_id).await {
        Ok(Some(user)) => Some(user.name.unwrap_or(user.login)),
        _ => None,
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
