//! Task HTTP handlers — coordinate repository calls + enrichment.

use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::Json;

use moatless_events::{SandboxEventPayload, SandboxStatus, TaskSubType};

use super::dao::{TaskFilter, TaskPatch};
use super::models::Task;
use super::relations::{CreatedByParent, TaskRelationType};
use crate::access_control::{
    ensure_can_control_task, ensure_can_use_repository, resolve_scope_for_create,
};
use crate::extractors::AuthUser;
use crate::shared_models::{AgentMode, AgentType};
use crate::state::AppState;
use crate::task::models::{SandboxDesiredState, SandboxStopReason, TaskStatus, Visibility};
use moatless_backend_core::scope::{Scope, ScopedResource};

use crate::repository::models::{RepositoryKind, RepositoryProvider};

use crate::bindings::dao::{get_bindings_for_tasks, PgTaskBindingRepository};
use crate::bindings::schemas::TaskBindingSummary;

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
    let scope_ctx = auth.scope_context();

    // A task accepts `repository_id` and/or `workspace_id`. When a workspace is
    // supplied it drives the task's repositories (workspace wins): the effective
    // repository is the workspace's primary placement. When only `repository_id`
    // is given, the single-repo path is unchanged and the shadow workspace is
    // derived downstream. At least one of the two is required.
    let workspace = match req.workspace_id.as_deref() {
        Some(workspace_id) => Some(
            state
                .workspaces
                .find_by_id_scoped(workspace_id, &scope_ctx)
                .await?
                .ok_or_else(|| TaskError::NotFound(format!("workspace {workspace_id}")))?,
        ),
        None => None,
    };

    let repository_id = match &workspace {
        Some(ws) => {
            state
                .workspaces
                .find_primary_repo(&ws.id)
                .await?
                .ok_or_else(|| {
                    TaskError::BadRequest(format!("Workspace {} has no primary repository", ws.id))
                })?
                .repository_id
        }
        None => req.repository_id.clone().ok_or_else(|| {
            TaskError::BadRequest("repositoryId or workspaceId is required".to_string())
        })?,
    };

    // Validate repository exists and is usable by this caller.
    let repo = state
        .repositories
        .find_by_id(&repository_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(repository_id.clone()))?;
    ensure_can_use_repository(
        &scope_ctx,
        &ScopedResource::borrowed(
            repo.scope,
            repo.team_id.as_deref(),
            repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
        ),
    )?;

    // Validate name if provided
    if let Some(ref name) = req.name {
        validate_task_name(name)?;
    }

    let is_template = repo.kind == RepositoryKind::Template;

    let task_id = uuid::Uuid::new_v4().to_string();
    let short_id = generate_unique_short_id(&state).await?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let agent_type = req
        .agent_type
        .unwrap_or_else(|| AgentType(AgentType::CLAUDE_CODE.into()));

    let (task_scope, task_team_id, task_owner_user_id) = if repo.scope == Scope::Team {
        (Scope::Team, repo.team_id.clone(), None)
    } else {
        let resolved_scope =
            resolve_scope_for_create(&state, &auth, Some(Scope::User), None).await?;
        (
            resolved_scope.scope,
            Some(resolved_scope.team_id),
            resolved_scope.owner_user_id,
        )
    };

    // Fast-fail codex tasks with no usable credential: a missing connection
    // or permanent refresh failure would otherwise surface minutes later as
    // a sandbox that dies on sign-in. Transient refresh errors (upstream
    // outage) do NOT block creation — the sandbox retries with the stored
    // blob.
    if agent_type.as_str() == "codex" {
        if let Err(crate::credentials::CredentialError::NotFound(reason)) = state
            .credentials_svc
            .ensure_fresh_codex_for(&auth.user_id, task_team_id.as_deref(), None)
            .await
        {
            tracing::info!(
                user_id = %auth.user_id,
                reason = %reason,
                "Rejecting codex task creation: no usable Codex credential"
            );
            return Err(TaskError::BadRequest(
                "Your Codex connection is missing or has expired. Reconnect Codex in Settings before creating a Codex task.".to_string(),
            ));
        }
    }

    let row = Task {
        task_id: task_id.clone(),
        short_id: Some(short_id),
        created_at: now.clone(),
        last_message_at: now,
        agent_type,
        repository_id: repository_id.clone(),
        status: TaskStatus::Open,
        total_cost_usd: 0.0,
        site_url: None,
        template_id: if is_template {
            Some(repository_id.clone())
        } else {
            None
        },
        vibe: if is_template { Some(true) } else { None },
        name: req.name.clone(),
        description: req.description.clone(),
        site_id: None,
        site_provider: None,
        branch: req.branch.clone().or(repo.default_branch.clone()),
        created_by: Some(auth.user_id.clone()),
        visibility: Visibility::Private,
        scope: task_scope,
        team_id: task_team_id,
        owner_user_id: task_owner_user_id,
        created_at_db: None,
        updated_at: None,
        append_system_prompt: None,
        sandbox_started_at: None,
        sandbox_desired_state: SandboxDesiredState::Running,
        sandbox_stop_reason: None,
        sandbox_desired_state_updated_at: chrono::Utc::now(),
        headless: req.headless.unwrap_or(false),
        ttl_minutes: state.config.sandbox.idle_timeout_minutes as i32,
        inactive_ttl_days: state.config.sandbox.inactive_task_timeout_days as i32,
        trigger_id: None,
        workspace_id: req.workspace_id.clone(),
        disabled_servers: None,
    };

    let parent = req.parent_task_id.as_deref().map(|p| CreatedByParent {
        parent_task_id: p.to_string(),
        context: serde_json::json!({ "via": "create_task_request" }),
    });

    insert_and_provision_task(
        &state,
        &auth,
        &row,
        req.message.as_deref(),
        req.model.as_deref(),
        req.agent_mode.as_ref(),
        &req.file_ids,
        &req.contexts,
        parent.as_ref(),
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
    parent: Option<&CreatedByParent>,
) -> Result<Json<CreateTaskResponse>, TaskError> {
    let task_id = &row.task_id;

    // Pre-flight: validate the GitHub token before the task row is written, so
    // an unusable credential rejects the request outright rather than leaving
    // a task stranded in Error.
    let repo = state
        .repositories
        .find_repository_by_id(&row.repository_id)
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(row.repository_id.clone()))?;

    let validator = crate::sandbox::preflight::RealGithubValidator;
    if let Err(e) =
        crate::sandbox::preflight::preflight_validate_github_token(state, row, &repo, &validator)
            .await
    {
        tracing::warn!(task_id = %task_id, error = %e, "Sandbox preflight failed");
        return Err(TaskError::Forbidden(e.user_message()));
    }

    state.tasks.insert_task(row).await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, repository_id = %row.repository_id, "Task created");

    if let Some(parent) = parent {
        crate::task::relations::record_created_by(state.task_relations.as_ref(), task_id, parent)
            .await;
    }

    // Always provision sandbox
    match crate::sandbox::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
        state.workspaces.as_ref(),
        task_id,
    )
    .await
    {
        Ok(request) => match state.sandbox.provision(task_id, &request).await {
            Ok(_response) => {
                tracing::info!(task_id = %task_id, "Sandbox provisioning started (initializing)");
            }
            Err(e) => {
                tracing::warn!(task_id = %task_id, error = %e, "Failed to provision sandbox");
                if let Some(ref events) = state.events {
                    events
                        .publish_sandbox_event_with_payload(
                            task_id,
                            SandboxStatus::Error,
                            &SandboxEventPayload {
                                message: format!("Failed to provision sandbox: {e}"),
                                error: Some(e.to_string()),
                                ..Default::default()
                            },
                        )
                        .await;
                }
            }
        },
        Err(e) => {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to build provision request");
            if let Some(ref events) = state.events {
                events
                    .publish_sandbox_event_with_payload(
                        task_id,
                        SandboxStatus::Error,
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
            state,
            task_id,
            msg,
            model,
            agent_mode,
            file_ids,
            contexts,
            true,
            None,
            None,
            None,
            None,
            None,
            false,
            Some(&auth.user_id),
        )
        .await
        {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to send initial message");
        }
    }

    let task_response = enrich_task(row, state, &auth.user_id).await?;

    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                task_id,
                TaskSubType::Created,
                Some(row.agent_type.as_str()),
                serde_json::json!({ "task": &task_response }),
            )
            .await;
    }

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
        .get_read_receipts(&auth.user_id, &task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve read receipts");
            Default::default()
        });

    // Batch resolve which of these tasks the requesting user follows (single
    // query). Drives the follow indicator; best-effort — empty on error.
    let followed = state
        .tasks
        .get_followed(&auth.user_id, &task_ids)
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

            let bindings: Vec<TaskBindingSummary> = all_bindings
                .iter()
                .filter(|c| c.task_id == row.task_id)
                .map(|c| TaskBindingSummary {
                    trigger_id: c.trigger_id.clone(),
                    adapter_kind: match c.provider.as_str() {
                        "github" | "github_pr" => moatless_api_schemas::enums::AdapterKind::Github,
                        "linear" => moatless_api_schemas::enums::AdapterKind::Linear,
                        "telegram" => moatless_api_schemas::enums::AdapterKind::Telegram,
                        "webhook" => moatless_api_schemas::enums::AdapterKind::Webhook,
                        _ => moatless_api_schemas::enums::AdapterKind::Slack,
                    },
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

            let mut resp =
                build_task_response(row, tags, display_name, repo_name, bindings, relations);
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

    let resp = enrich_task(&row, &state, &auth.user_id).await?;
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

    let (messages, _, blocks) = state.messages.get_all(&task_id).await?;
    let turn_metrics = state
        .task_turns
        .list_turn_metrics(&task_id)
        .await
        .unwrap_or_default();
    Ok(Json(super::statistics::build_task_statistics(
        &row,
        &messages,
        &blocks,
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
        validate_task_name(name)?;
    }

    let existing = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    ensure_can_control_task(&state, &auth, &task_resource(&existing)).await?;

    state
        .tasks
        .update_task(
            &task_id,
            &TaskPatch {
                name: req.name.clone(),
                description: req.description.clone(),
                ..Default::default()
            },
        )
        .await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task metadata updated");

    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                &task_id,
                TaskSubType::MetadataUpdated,
                Some(existing.agent_type.as_str()),
                serde_json::json!({
                    "name": req.name,
                    "description": req.description,
                }),
            )
            .await;
    }

    let row = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&row, &state, &auth.user_id).await?;
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
async fn deactivate_task_bindings(state: &AppState, task_id: &str) {
    match PgTaskBindingRepository(state.db.clone())
        .deactivate_by_task(task_id)
        .await
    {
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
    let existing = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    ensure_can_control_task(&state, &auth, &task_resource(&existing)).await?;

    // Stop the agent gracefully before tearing down the workspace so the pod
    // exits promptly instead of riding its full termination grace until SIGKILL
    // (exit 137). Best-effort — never blocks teardown.
    crate::sandbox::notify::stop_agent_for_task(&state, &task_id).await;

    // Cleanup workspace (best-effort). Permanent deletion removes S3 storage
    // too — never retain here.
    if let Err(e) = state.sandbox.cleanup(&task_id, false).await {
        tracing::warn!(task_id = %task_id, error = %e, "Failed to cleanup workspace on delete");
    }

    // Stop external listeners. `task_bindings.task_id` is not an FK, so rows
    // don't cascade on delete — deactivate them so inbound routing stops.
    deactivate_task_bindings(&state, &task_id).await;

    state.tasks.delete_task(&task_id).await?;
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
        .mark_task_read(&auth.user_id, &task_id, chrono::Utc::now())
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

    state.tasks.follow_task(&auth.user_id, &task_id).await?;
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

    state.tasks.unfollow_task(&auth.user_id, &task_id).await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task unfollowed");

    Ok(Json(FollowTaskResponse { following: false }))
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
    let row = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    ensure_can_control_task(&state, &auth, &task_resource(&row)).await?;

    let mut visited = HashSet::new();
    close_task_tree(&state, &task_id, &mut visited).await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task closed");

    let updated = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&updated, &state, &auth.user_id).await?;
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
    let Some(row) = state.tasks.find_by_id_internal(task_id).await? else {
        tracing::warn!(task_id, "close cascade: task vanished mid-close");
        return Ok(());
    };

    if row.status == TaskStatus::Closed {
        return Ok(());
    }

    // Stop the agent gracefully before tearing down the sandbox so the pod
    // exits promptly instead of riding its full termination grace until SIGKILL
    // (exit 137). Best-effort — never blocks teardown.
    crate::sandbox::notify::stop_agent_for_task(state, task_id).await;

    state
        .tasks
        .update_task(
            task_id,
            &TaskPatch {
                status: Some(TaskStatus::Closed),
                sandbox_desired_state: Some(SandboxDesiredState::Removed),
                sandbox_stop_reason: Some(Some(SandboxStopReason::Cleanup)),
                ..Default::default()
            },
        )
        .await?;

    // Cleanup sandbox (best-effort). Preserve S3 storage when configured so the
    // task can later be revived via reopen.
    let retain_storage = state.config.sandbox.retain_storage_on_close;
    if let Err(e) = state.sandbox.cleanup(task_id, retain_storage).await {
        tracing::warn!(task_id, error = %e, "Failed to cleanup sandbox on close");
    }

    // A closed task is terminal — stop its external listeners so inbound
    // comments don't resurrect it.
    deactivate_task_bindings(state, task_id).await;

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
    let row = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    ensure_can_control_task(&state, &auth, &task_resource(&row)).await?;

    if row.status != TaskStatus::Closed {
        return Err(TaskError::BadRequest(format!(
            "Cannot reopen task in '{}' status (only closed tasks can be reopened)",
            row.status
        )));
    }

    state
        .tasks
        .update_task(
            &task_id,
            &TaskPatch {
                status: Some(TaskStatus::Open),
                ..Default::default()
            },
        )
        .await?;

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
    let row = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    ensure_can_control_task(&state, &auth, &task_resource(&row)).await?;

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
        .find_repository_by_id(&row.repository_id)
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(row.repository_id.clone()))?;

    let validator = crate::sandbox::preflight::RealGithubValidator;
    if let Err(e) =
        crate::sandbox::preflight::preflight_validate_github_token(&state, &row, &repo, &validator)
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
    // `config.json` + `loops/` before provisioning so the sandbox and trigger
    // tables pick up the latest committed settings. Never fails the start.
    maybe_sync_repo_config_on_start(&state, &auth, &repo).await;

    state
        .tasks
        .update_task(
            &task_id,
            &TaskPatch {
                sandbox_desired_state: Some(SandboxDesiredState::Running),
                sandbox_stop_reason: Some(None),
                ..Default::default()
            },
        )
        .await?;

    let request = crate::sandbox::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
        state.workspaces.as_ref(),
        &task_id,
    )
    .await
    .map_err(|e| TaskError::Internal(e.to_string()))?;

    state
        .sandbox
        .provision(&task_id, &request)
        .await
        .map_err(|e| TaskError::Internal(e.to_string()))?;

    tracing::info!(task_id = %task_id, "Sandbox provision initiated (initializing)");
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Best-effort auto-sync of a repo's `.moatless/` config + loops when a task
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
        crate::auth::credentials::resolve_state_github_token_for(
            state,
            &auth.user_id,
            &auth.team_ids,
            None,
        )
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(repository_id = %repo.id, error = %e, "Config auto-sync: failed to resolve GitHub token; probing unauthenticated");
                None
            })
    } else {
        match state
            .provider_tokens
            .find_provider_token(&auth.user_id, provider.as_str())
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

    match crate::repository::handlers::sync_repo_config(state, repo, auth).await {
        Ok((_, summary)) => {
            tracing::info!(
                repository_id = %repo.id,
                created = summary.created,
                updated = summary.updated,
                removed = summary.removed,
                skipped = summary.skipped.len(),
                "Config auto-sync on task start complete"
            );
        }
        Err(e) => {
            tracing::warn!(repository_id = %repo.id, error = %e, "Config auto-sync on task start failed");
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
    let task = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    ensure_can_control_task(&state, &auth, &task_resource(&task)).await?;

    // Stop the agent process via gRPC (keeps sandbox pod running). When this
    // call succeeds the sandbox publishes `agent.stopped` itself, so we must
    // not publish it again or the event fan-out doubles per request.
    let mut sandbox_published_stop = false;
    match state.sandbox.sandbox_client(&task_id).await {
        Ok(client) => match client.stop_agent().await {
            Ok(()) => {
                sandbox_published_stop = true;
            }
            Err(e) => {
                tracing::warn!(task_id = %task_id, error = %e, "Failed to stop agent via gRPC");
            }
        },
        Err(e) => {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to get sandbox client for agent stop");
        }
    }

    // Fallback publish only when the sandbox couldn't publish for us
    // (Docker container killed mid-call, pod gone, gRPC unreachable).
    // `handle_agent_stopped` is the sole owner of `taskTurn.stopped` —
    // never publish it from here.
    if !sandbox_published_stop {
        if let Some(ref events) = state.events {
            events
                .publish_agent_event(
                    &task_id,
                    moatless_events::AgentSubType::Stopped,
                    None,
                    serde_json::json!({}),
                )
                .await;
        }
    }

    // Update task status and record that this was an intentional user stop.
    state
        .tasks
        .update_task(
            &task_id,
            &TaskPatch {
                status: Some(TaskStatus::Open),
                sandbox_desired_state: Some(SandboxDesiredState::Stopped),
                sandbox_stop_reason: Some(Some(SandboxStopReason::User)),
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
    let existing = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    ensure_can_control_task(&state, &auth, &task_resource(&existing)).await?;

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
    let existing = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    ensure_can_control_task(&state, &auth, &task_resource(&existing)).await?;

    state
        .tasks
        .update_task(
            &task_id,
            &TaskPatch {
                visibility: Some(req.visibility),
                ..Default::default()
            },
        )
        .await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, visibility = %req.visibility, "Task visibility updated");

    let row = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = enrich_task(&row, &state, &auth.user_id).await?;
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

    let existing = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    ensure_can_control_task(&state, &auth, &task_resource(&existing)).await?;

    state
        .tasks
        .update_task(
            &task_id,
            &TaskPatch {
                ttl_minutes: req.ttl_minutes,
                inactive_ttl_days: req.inactive_ttl_days,
                ..Default::default()
            },
        )
        .await?;
    tracing::info!(
        task_id = %task_id,
        user_id = %auth.user_id,
        ttl_minutes = ?req.ttl_minutes,
        inactive_ttl_days = ?req.inactive_ttl_days,
        "Task TTL updated"
    );

    let row = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
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

    let repo_name = state
        .repositories
        .find_repository_by_id(&row.repository_id)
        .await?
        .map(|r| r.name);

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

    let mut resp = build_task_response(
        row,
        tag_responses,
        display_name,
        repo_name,
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
fn build_task_response(
    row: &Task,
    tags: Vec<TagResponse>,
    display_name: Option<String>,
    repo_name: Option<String>,
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
        owner_user_id: row.owner_user_id.clone(),
        parent_task_id,
        relations,
        append_system_prompt: row.append_system_prompt.clone(),
        tags,
        bindings,
        ttl_minutes: row.ttl_minutes,
        inactive_ttl_days: row.inactive_ttl_days,
        sandbox_started_at: row.sandbox_started_at.map(|t| t.to_rfc3339()),
        sandbox_desired_state: row.sandbox_desired_state,
        sandbox_stop_reason: row.sandbox_stop_reason,
        sandbox_desired_state_updated_at: row.sandbox_desired_state_updated_at.to_rfc3339(),
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

/// Validate task name (URL-friendly: alphanumeric + dashes).
fn validate_task_name(name: &str) -> Result<(), TaskError> {
    if name.is_empty() {
        return Err(TaskError::BadRequest(
            "Task name cannot be empty".to_string(),
        ));
    }
    if name.len() > 200 {
        return Err(TaskError::BadRequest(
            "Task name too long (max 200)".to_string(),
        ));
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
    Err(TaskError::Internal(
        "Failed to generate unique short ID".to_string(),
    ))
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

/// Resolve external bindings for a single task (used by enrich_task).
async fn resolve_task_bindings(state: &AppState, task_id: &str) -> Vec<TaskBindingSummary> {
    let bindings = match crate::bindings::dao::PgTaskBindingRepository(state.db.clone())
        .find_by_task_id(task_id)
        .await
    {
        Ok(bindings) => bindings,
        Err(e) => {
            tracing::warn!(error = %e, task_id = %task_id, "Failed to resolve task bindings");
            return vec![];
        }
    };

    let mut summaries = Vec::new();
    for binding in &bindings {
        let trigger = if let Some(ref trigger_id) = binding.trigger_id {
            crate::triggers::rule_dao::PgTaskTriggerRouteRepository(state.db.clone())
                .find_by_id(trigger_id)
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
