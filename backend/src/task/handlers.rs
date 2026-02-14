//! Task HTTP handlers — coordinate DAO calls + enrichment.

use axum::extract::{Path, Query, State};
use axum::Json;

use moatless_events::TaskSubType;

use crate::auth::users;
use crate::repository::dao as repository_dao;
use crate::tag::dao as tag_dao;
use super::dao as task_dao;
use super::dao::TaskPatch;
use super::models::Task;
use crate::extractors::AuthUser;
use crate::state::AppState;
use crate::task::models::{TaskStatus, Visibility};

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
        (status = 404, description = "Repository not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn create_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateTaskRequest>,
) -> Result<Json<CreateTaskResponse>, TaskError> {
    // Validate repository exists
    let repo = repository_dao::find_repository_by_id(&state.db, &req.repository_id)
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(req.repository_id.clone()))?;

    // Validate name if provided
    if let Some(ref name) = req.name {
        validate_task_name(name)?;
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let short_id = generate_unique_short_id(&state.db).await?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let agent_type = req.agent_type.unwrap_or_else(|| "claude-code".to_string());

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
        template_id: None,
        vibe: None,
        name: req.name.clone(),
        description: req.description.clone(),
        site_id: None,
        site_provider: None,
        branch: req.branch.clone().or(repo.default_branch.clone()),
        created_by: Some(auth.user_id.clone()),
        visibility: Visibility::Private,
        created_at_db: None,
    };

    task_dao::insert_task(&state.db, &row).await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, repository_id = %req.repository_id, "Task created");

    // Emit task event
    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                &task_id,
                TaskSubType::Open,
                Some(&row.agent_type),
                serde_json::json!({}),
            )
            .await;
    }

    // Always provision sandbox
    match state.sandbox.start(&task_id).await {
        Ok(()) => {
            tracing::info!(task_id = %task_id, "Sandbox provisioning started (initializing)");
        }
        Err(e) => {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to provision sandbox");
        }
    }

    // If a message was provided, send it to the agent
    if let Some(message) = req.message {
        if let Err(e) = crate::message::send::send_message_flow(
            &state,
            &task_id,
            &message,
            req.model.as_deref(),
            req.agent_mode.as_deref(),
            &req.file_ids,
            &req.contexts,
        )
        .await
        {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to send initial message");
        }
    }

    let task_response = enrich_task(&row, &state, &auth.user_id).await?;

    Ok(Json(CreateTaskResponse {
        task_id: row.task_id.clone(),
        status: row.status,
        task: task_response,
    }))
}

/// Create a new vibe task from a template.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/vibe",
    tag = "tasks",
    request_body = CreateVibeTaskRequest,
    responses(
        (status = 200, description = "Vibe task created", body = CreateTaskResponse),
        (status = 404, description = "Template not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn create_vibe_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateVibeTaskRequest>,
) -> Result<Json<CreateTaskResponse>, TaskError> {
    // Find the template repository
    let repo = repository_dao::find_repository_by_id(&state.db, &req.template_id)
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(req.template_id.clone()))?;

    let task_id = uuid::Uuid::new_v4().to_string();
    let short_id = generate_unique_short_id(&state.db).await?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let row = Task {
        task_id: task_id.clone(),
        short_id: Some(short_id),
        created_at: now.clone(),
        last_message_at: now,
        agent_type: "claude-code".to_string(),
        repository_id: repo.id.clone(),
        status: TaskStatus::Open,
        total_cost_usd: 0.0,
        site_url: None,
        template_id: Some(req.template_id.clone()),
        vibe: Some(true),
        name: req.name.clone(),
        description: None,
        site_id: None,
        site_provider: None,
        branch: repo.default_branch.clone(),
        created_by: Some(auth.user_id.clone()),
        visibility: Visibility::Private,
        created_at_db: None,
    };

    task_dao::insert_task(&state.db, &row).await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, template_id = %req.template_id, "Vibe task created");

    if let Some(ref events) = state.events {
        events
            .publish_task_event(&task_id, TaskSubType::Open, Some("claude-code"), serde_json::json!({}))
            .await;
    }

    // Always provision sandbox
    match state.sandbox.start(&task_id).await {
        Ok(()) => {
            tracing::info!(task_id = %task_id, "Sandbox provisioning started (initializing)");
        }
        Err(e) => {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to provision sandbox");
        }
    }

    if let Some(message) = req.message {
        if let Err(e) = crate::message::send::send_message_flow(
            &state,
            &task_id,
            &message,
            None,
            None,
            &[],
            &[],
        )
        .await
        {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to send vibe initial message");
        }
    }

    let task_response = enrich_task(&row, &state, &auth.user_id).await?;

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

    let (rows, total) = task_dao::list_tasks(
        &state.db,
        &auth.user_id,
        limit,
        offset,
        query.status.as_deref(),
        query.repository_id.as_deref(),
        query.created_by.as_deref(),
        query.tag.as_deref(),
    )
    .await?;

    let task_ids: Vec<String> = rows.iter().map(|r| r.task_id.clone()).collect();
    let all_tags = tag_dao::get_tags_for_tasks(&state.db, &task_ids).await?;

    // Batch resolve user display names
    let creator_ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r.created_by.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let user_names = resolve_user_display_names(&state, &creator_ids).await;

    // Batch resolve repository names
    let repo_ids: Vec<String> = rows
        .iter()
        .map(|r| r.repository_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let repo_names = resolve_repo_names(&state, &repo_ids).await;

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
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn get_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<TaskResponse>, TaskError> {
    let row = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
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
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
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

    let _existing = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    task_dao::update_task(
        &state.db,
        &task_id,
        &TaskPatch {
            name: req.name,
            description: req.description,
            ..Default::default()
        },
    )
    .await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task metadata updated");

    let row = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
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
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn delete_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    let _existing = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    // Cleanup workspace (best-effort)
    if let Err(e) = state.sandbox.cleanup(&task_id).await {
        tracing::warn!(task_id = %task_id, error = %e, "Failed to cleanup workspace on delete");
    }

    task_dao::delete_task(&state.db, &task_id).await?;
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
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn close_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<TaskResponse>, TaskError> {
    let row = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    // Cleanup sandbox (best-effort)
    if let Err(e) = state.sandbox.cleanup(&task_id).await {
        tracing::warn!(task_id = %task_id, error = %e, "Failed to cleanup sandbox on close");
    }
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task closed");

    task_dao::update_task(
        &state.db,
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
                Some(&row.agent_type),
                serde_json::json!({}),
            )
            .await;
    }

    let updated = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
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
        (status = 400, description = "Invalid task state", body = crate::sandbox::schemas::ErrorBody),
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn start_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    let row = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    if row.status != TaskStatus::Open && row.status != TaskStatus::Error {
        return Err(TaskError::BadRequest(format!(
            "Cannot start task in '{}' status",
            row.status
        )));
    }

    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Starting task");

    state.sandbox.start(&task_id).await
        .map_err(|e| TaskError::Internal(e.to_string()))?;

    tracing::info!(task_id = %task_id, "Sandbox start initiated (initializing)");
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
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn stop_task(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    let _task = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    // Stop the sandbox agent process
    if let Err(e) = state.sandbox.stop(&task_id).await {
        tracing::warn!(task_id = %task_id, error = %e, "Failed to stop sandbox agent");
    }

    // Update task status
    task_dao::update_task(
        &state.db,
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
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn set_task_tags(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SetTagsRequest>,
) -> Result<Json<Vec<TagResponse>>, TaskError> {
    let _existing = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let tags = tag_dao::find_or_create_tags(&state.db, &req.tags).await?;
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();
    tag_dao::set_task_tags(&state.db, &task_id, &tag_ids).await?;
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
        (status = 400, description = "Invalid visibility value", body = crate::sandbox::schemas::ErrorBody),
        (status = 404, description = "Task not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn update_visibility(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateVisibilityRequest>,
) -> Result<Json<TaskResponse>, TaskError> {
    let _existing = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    task_dao::update_task(
        &state.db,
        &task_id,
        &TaskPatch {
            visibility: Some(req.visibility),
            ..Default::default()
        },
    )
    .await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, visibility = %req.visibility, "Task visibility updated");

    let row = task_dao::find_by_id(&state.db, &task_id, &auth.user_id)
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
    let tags = tag_dao::get_tags_for_task(&state.db, &row.task_id).await?;
    let tag_responses: Vec<TagResponse> = tags
        .iter()
        .map(|t| TagResponse {
            tag_id: t.tag_id.clone(),
            name: t.name.clone(),
        })
        .collect();

    let repo_name = repository_dao::find_repository_by_id(&state.db, &row.repository_id)
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
    row.agent_type.clone()
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
async fn generate_unique_short_id(pool: &crate::db::DbPool) -> Result<String, TaskError> {
    for _ in 0..10 {
        let id = nanoid::nanoid!(8, &nanoid::alphabet::SAFE);
        if !task_dao::exists_by_short_id(pool, &id).await? {
            return Ok(id);
        }
    }
    Err(TaskError::Internal("Failed to generate unique short ID".to_string()))
}

/// Resolve display names for multiple user IDs.
async fn resolve_user_display_names(
    state: &AppState,
    user_ids: &[String],
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for uid in user_ids {
        if let Some(name) = resolve_single_user_display_name(state, uid).await {
            map.insert(uid.clone(), name);
        }
    }
    map
}

/// Resolve a single user's display name.
async fn resolve_single_user_display_name(state: &AppState, user_id: &str) -> Option<String> {
    match users::get_user(&state.db, user_id).await {
        Ok(Some(user)) => Some(user.name.unwrap_or(user.login)),
        _ => None,
    }
}

/// Resolve repository names for multiple repo IDs.
async fn resolve_repo_names(
    state: &AppState,
    repo_ids: &[String],
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for rid in repo_ids {
        if let Ok(Some(repo)) = repository_dao::find_repository_by_id(&state.db, rid).await {
            map.insert(rid.clone(), repo.name);
        }
    }
    map
}
