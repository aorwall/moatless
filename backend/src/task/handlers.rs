//! Task HTTP handlers — coordinate repository calls + enrichment.

use axum::extract::{Path, Query, State};
use axum::Json;

use moatless_events::{SandboxEventPayload, SandboxStatus, SandboxSubType, TaskSubType};

use crate::access_control::{
    ensure_can_control_task, ensure_can_use_repository, resolve_scope_for_create,
};
use crate::sandbox::auto_provision::get_server_configs_for_task;
use crate::sandbox::models::AgentState;

use super::dao::{TaskFilter, TaskPatch};
use super::models::Task;
use crate::extractors::AuthUser;
use crate::scope::{Scope, ScopedResource};
use crate::shared_models::{AgentMode, AgentType};
use crate::state::AppState;
use crate::task::models::{TaskStatus, Visibility};

use crate::repository::models::{RepositoryKind, RepositoryProvider};

use crate::channel::dao::bindings::get_channels_for_tasks;
use crate::channel::schemas::TaskChannelResponse;

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
    let scope_ctx = auth.scope_context();

    // Validate repository exists and is usable by this caller.
    let repo = state
        .repositories
        .find_by_id(&req.repository_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(req.repository_id.clone()))?;
    ensure_can_use_repository(
        &scope_ctx,
        &ScopedResource::borrowed(
            repo.scope,
            repo.team_id.as_deref(),
            repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
        ),
    )?;

    // Verify GitHub repository access for non-template repos
    if repo.provider == Some(RepositoryProvider::Github) && repo.kind != RepositoryKind::Template {
        if let Some(ref remote_url) = repo.remote_url {
            if let Some((owner, repo_name)) =
                crate::repository::handlers::parse_github_owner_repo(remote_url)
            {
                // Resolve a fresh GitHub token via the credentials service so
                // GitHub App user-tokens get refreshed before we hit GitHub.
                // Without this, an expired access_token left in
                // `provider_tokens` (App tokens expire every ~8h) leaks
                // straight to `check_github_repo_access` and the user gets
                // an opaque 401 even though their setup is fine — the
                // sandbox-provision path (which goes through the same
                // service) keeps working in parallel.
                let access_token = if let Ok(t) = state
                    .credentials_svc
                    .ensure_fresh(&auth.user_id, "GITHUB_TOKEN")
                    .await
                {
                    t
                } else {
                    // Fall back to a team-level PAT if the user has
                    // none — preserves the previous team-token path.
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
                    match team_token {
                        Some(t) => t.access_token,
                        None => {
                            return Err(TaskError::Forbidden(
                                "GitHub account not connected. Please connect your GitHub account to create tasks for this repository.".to_string(),
                            ));
                        }
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
        template_id: if is_template {
            Some(req.repository_id.clone())
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
        parent_task_id: req.parent_task_id.clone(),
        append_system_prompt: None,
        sandbox_started_at: None,
        headless: req.headless.unwrap_or(false),
        ttl_minutes: state.config.sandbox.idle_timeout_minutes as i32,
        task_template_id: req.task_template_id.clone(),
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
        req.command_name.as_deref(),
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
    command_name: Option<&str>,
) -> Result<Json<CreateTaskResponse>, TaskError> {
    let task_id = &row.task_id;

    state.tasks.insert_task(row).await?;
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, repository_id = %row.repository_id, "Task created");

    // Emit task event
    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                task_id,
                TaskSubType::Open,
                Some(row.agent_type.as_str()),
                serde_json::json!({}),
            )
            .await;
    }

    // Pre-flight: validate the GitHub token before any sandbox resources
    // are created. The task row already exists, so on failure we mark it
    // Error + publish a sandbox.error event and skip provisioning entirely.
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
        let msg = e.user_message();
        tracing::warn!(task_id = %task_id, error = %e, "Sandbox preflight failed");
        if let Err(db_err) = state
            .tasks
            .update_task(
                task_id,
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
                    task_id,
                    SandboxSubType::Error,
                    &SandboxEventPayload {
                        message: msg,
                        error: Some(e.to_string()),
                        lifecycle_event: Some("preflight_failed".to_string()),
                    },
                )
                .await;
        }
        let task_response = enrich_task(row, state, &auth.user_id).await?;
        return Ok(Json(CreateTaskResponse {
            task_id: row.task_id.clone(),
            status: TaskStatus::Error,
            task: task_response,
        }));
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
        },
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
            command_name,
            None,
            false,
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

    let filter = TaskFilter {
        scope: auth.scope_context(),
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

    // Batch resolve channels for tasks (single query instead of N+1)
    let all_channels = get_channels_for_tasks(&state.db, &task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve task channels");
            vec![]
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

            let channels: Vec<TaskChannelResponse> = all_channels
                .iter()
                .filter(|c| c.task_id == row.task_id)
                .map(|c| TaskChannelResponse {
                    trigger_id: c.trigger_id.clone(),
                    provider: match c.provider.as_str() {
                        "github" | "github_pr" => {
                            moatless_api_schemas::enums::IntegrationProvider::Github
                        }
                        "linear" => moatless_api_schemas::enums::IntegrationProvider::Linear,
                        "telegram" => moatless_api_schemas::enums::IntegrationProvider::Telegram,
                        "webhook" => moatless_api_schemas::enums::IntegrationProvider::Webhook,
                        _ => moatless_api_schemas::enums::IntegrationProvider::Slack,
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

            build_task_response(row, tags, display_name, repo_name, channels)
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
    let row = state
        .tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    ensure_can_control_task(&state, &auth, &task_resource(&row)).await?;

    // Cleanup sandbox (best-effort)
    if let Err(e) = state.sandbox.cleanup(&task_id).await {
        tracing::warn!(task_id = %task_id, error = %e, "Failed to cleanup sandbox on close");
    }
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task closed");

    state
        .tasks
        .update_task(
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
                    SandboxSubType::Error,
                    &SandboxEventPayload {
                        message: msg.clone(),
                        error: Some(e.to_string()),
                        lifecycle_event: Some("preflight_failed".to_string()),
                    },
                )
                .await;
        }
        return Err(TaskError::BadRequest(msg));
    }

    let request = crate::sandbox::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
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

    // Update task status
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

/// Update a task's idle TTL (in minutes). `0` disables the idle reaper for
/// this task — the sandbox stays alive until it's explicitly stopped.
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
    if req.ttl_minutes < 0 {
        return Err(TaskError::BadRequest(
            "ttl_minutes must be >= 0".to_string(),
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
                ttl_minutes: Some(req.ttl_minutes),
                ..Default::default()
            },
        )
        .await?;
    tracing::info!(
        task_id = %task_id,
        user_id = %auth.user_id,
        ttl_minutes = req.ttl_minutes,
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

    // Resolve channels for this task
    let channels = resolve_task_channels(state, &row.task_id).await;

    let mut resp = build_task_response(row, tag_responses, display_name, repo_name, channels);

    // Resolve live sandbox status (best-effort — leave None on failure)
    let server_configs = get_server_configs_for_task(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        &row.task_id,
    )
    .await;
    if let Ok(status) = state
        .sandbox
        .sandbox_status(&row.task_id, &server_configs)
        .await
    {
        resp.environment_status = Some(status.environment_status.as_str().to_string());
        resp.agent_state = Some(
            match status.agent_state {
                AgentState::Idle => "idle",
                AgentState::Running => "running",
                AgentState::None => "none",
            }
            .to_string(),
        );
        resp.is_running = Some(status.environment_status == SandboxStatus::Ready);
    }

    Ok(resp)
}

/// Build a TaskResponse from a row + pre-resolved tags/names/channels.
fn build_task_response(
    row: &Task,
    tags: Vec<TagResponse>,
    display_name: Option<String>,
    repo_name: Option<String>,
    channels: Vec<TaskChannelResponse>,
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
        owner_user_id: row.owner_user_id.clone(),
        parent_task_id: row.parent_task_id.clone(),
        append_system_prompt: row.append_system_prompt.clone(),
        tags,
        channels,
        environment_status: None,
        agent_state: None,
        is_running: None,
        ttl_minutes: row.ttl_minutes,
    }
}

fn task_resource(row: &Task) -> ScopedResource {
    ScopedResource::borrowed(
        row.scope,
        row.team_id.as_deref(),
        row.owner_user_id.as_deref().or(row.created_by.as_deref()),
    )
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

/// Resolve channels for a single task (used by enrich_task).
async fn resolve_task_channels(state: &AppState, task_id: &str) -> Vec<TaskChannelResponse> {
    let bindings = match crate::channel::dao::bindings::PgTaskBindingRepository(state.db.clone())
        .find_by_task_id(task_id)
        .await
    {
        Ok(bindings) => bindings,
        Err(e) => {
            tracing::warn!(error = %e, task_id = %task_id, "Failed to resolve task channels");
            return vec![];
        }
    };

    let mut channels = Vec::new();
    for binding in &bindings {
        let trigger = if let Some(ref trigger_id) = binding.trigger_id {
            crate::channel::dao::trigger_routes::PgTaskTriggerRouteRepository(state.db.clone())
                .find_by_id(trigger_id)
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        channels.push(crate::channel::schemas::task_channel_from_binding(
            binding,
            trigger.as_ref(),
        ));
    }
    channels
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
