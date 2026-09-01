//! Task HTTP handlers — coordinate repository calls + enrichment.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{Path, Query, State};

use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus};
use moatless_types::task::TaskSubType;

use super::creation::{ForkedFrom, TaskCreationService};
use super::dao::{TaskDao, TaskFilter, TaskPatch};
use super::fork::TaskForkService;
use super::models::Task;
use super::relations::TaskRelationDao;
use crate::credential_sync::CredentialSync;
use crate::message::dao::MessageDao;
use crate::message::send::MessageSendService;
use crate::tag::dao::TagDao;
use crate::task::models::{SandboxIntentView, TaskStatus};
use crate::task_turn::dao::TaskTurnDao;
use moatless_backend_core::sandbox::PreflightPort;
use moatless_backend_core::sandbox::SandboxLifecycle;
use moatless_backend_core::workspace::{RepositoryReader, WorkspaceResolver};
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_core::http_auth::AuthUser;
use moatless_types::access_control::{ScopeContext, ScopedResource};
use moatless_types::sandbox::ArchiveRetention;
use moatless_types::task::TaskRelationType;
use moatless_workspaces::workspace::git_sync::GitSyncService;

use moatless_api_schemas::bindings::TaskBindingSummary;
use moatless_backend_core::binding::TaskBindings;

use super::closer::{CloseEvent, TaskCloser};
use super::error::TaskError;
use super::presenter::{ResolvedNames, TaskPresenter, build_task_response};
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
        (status = 400, description = "Invalid request or unusable Codex credential", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "GitHub not connected, or no access to the repository", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Repository not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "seven ports, all forwarded: this handler is the outermost caller of a two-stage service pipeline — four to service::normalize, two to service::create_task, one to the presenter closure. Unlike the service-layer create_task this was split out of, there is no caller above a handler to hoist them to, so the width cannot move anywhere it would be narrower."
)]
pub async fn create_task(
    State(workspaces): State<Arc<dyn WorkspaceResolver>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
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
        workspaces.as_ref(),
        repositories.as_ref(),
        &credential_sync,
        &config,
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
        (status = 409, description = "A turn is in flight, or the turn has no agent turn id", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "the same two-stage create pipeline `create_task` forwards to, plus the fork service that resolves the cut and copies the conversation ahead of it. As there, no caller above a handler exists to hoist the ports to."
)]
pub async fn fork_task(
    State(workspaces): State<Arc<dyn WorkspaceResolver>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    State(credential_sync): State<Arc<CredentialSync>>,
    State(config): State<Arc<Config>>,
    State(task_creation): State<Arc<TaskCreationService>>,
    State(task_fork): State<Arc<TaskForkService>>,
    State(message_send): State<Arc<MessageSendService>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<ForkTaskRequest>,
) -> Result<Json<CreateTaskResponse>, TaskError> {
    let same_sandbox = req.same_sandbox.unwrap_or(false);
    TaskForkService::validate_checkout(same_sandbox, req.branch.as_deref())?;

    let user_id = auth.user_id.to_string();
    let scope_ctx = auth.scope_context();
    let (source, plan) = task_fork.plan(&scope_ctx, &task_id, req.at_turn).await?;

    // The fork's id is minted here rather than by the creation service: the
    // conversation is copied onto it before the Task row exists, which is what
    // puts the copied Messages behind the fork's own first message.
    let fork_task_id = uuid::Uuid::new_v4().to_string();
    let copied = task_fork.copy_conversation(&plan, &fork_task_id).await?;

    // Everything the fork runs against comes from the Task it branched off. A
    // fork is deliberately created unnamed and undescribed: "Fork of X" says
    // where it came from, not what it is doing, and the agent sets both with
    // `moat set-name` once it knows.
    let create = CreateTaskRequest {
        repository_id: Some(source.repository_id.clone()),
        workspace_id: source.workspace_id.clone(),
        sandbox_id: same_sandbox.then(|| source.sandbox_id.clone()).flatten(),
        checkout_mode: same_sandbox.then_some(moatless_types::task::CheckoutMode::Shared),
        agent_type: Some(source.agent_type.clone()),
        name: None,
        description: None,
        branch: req.branch.clone().or_else(|| source.branch.clone()),
        message: req.message.clone(),
        model: req.model.clone().or_else(|| source.model.clone()),
        agent_mode: req.agent_mode.clone(),
        effort: req.effort.clone(),
        contexts: Vec::new(),
        file_ids: Vec::new(),
        parent_task_id: None,
        headless: None,
        skills: req.skills.clone(),
    };

    // The conversation now sits under `fork_task_id` with no Task row to own it,
    // and nothing cascades it away: `task_messages` and `task_activities` carry
    // no foreign key to `tasks`, which is what let the copy land ahead of the
    // create. So every failure from here — a rejected credential in `normalize`,
    // a preflight refusal, a Sandbox join that never lands — has to take the copy
    // back down, or it outlives the create that never completed. The create runs
    // inside a block whose error triggers exactly that before it propagates.
    // `enriched` is filled inside and read after for the response, so it lives
    // outside the block.
    let enriched: Arc<Mutex<Option<TaskResponse>>> = Arc::new(Mutex::new(None));
    let create_result = async {
        let mut new_task = service::normalize(
            workspaces.as_ref(),
            repositories.as_ref(),
            &credential_sync,
            &config,
            &scope_ctx,
            &user_id,
            service::CreateTaskIntent {
                request: create,
                task_id: Some(fork_task_id.clone()),
                provision: true,
                first_message_uuid: None,
            },
        )
        .await?;
        new_task.tag_ids = task_fork.source_tag_ids(&task_id).await;
        new_task.forked_from = Some(ForkedFrom {
            source_task_id: task_id.clone(),
            at_turn: plan.at_turn,
            session: plan.session.clone(),
        });

        // Enrich once, as `create_task` does: the event payload and the HTTP
        // response are the same object, and `enrich_task` is ~6 queries.
        let presenter_ev = task_presenter.clone();
        let uid_ev = user_id.clone();
        let enriched_ev = enriched.clone();
        service::create_task(&task_creation, message_send, new_task, move |task| async move {
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
        })
        .await
    }
    .await;

    let outcome = match create_result {
        Ok(outcome) => outcome,
        Err(error) => {
            // The Task never came to exist; take its copied conversation with it
            // rather than leave rows behind a Task that is not there.
            task_fork.discard_conversation(&fork_task_id).await;
            return Err(error);
        }
    };

    tracing::info!(
        task_id = %fork_task_id,
        source_task_id = %task_id,
        at_turn = ?plan.at_turn,
        messages = copied.messages,
        activities = copied.activities,
        contexts = copied.contexts,
        same_sandbox,
        "Task forked"
    );

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
    State(tasks): State<Arc<dyn TaskDao>>,
    State(tags): State<Arc<dyn TagDao>>,
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

    // Batch resolve workspace names for the tasks that have a workspace.
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
    State(tasks): State<Arc<dyn TaskDao>>,
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
    State(tasks): State<Arc<dyn TaskDao>>,
    State(messages_dao): State<Arc<dyn MessageDao>>,
    State(task_turns): State<Arc<dyn TaskTurnDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<TaskStatisticsResponse>, TaskError> {
    let row = tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let (messages, _, activities) = messages_dao.get_all(&task_id).await?;
    let turn_metrics = task_turns
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn update_task_metadata(
    State(tasks): State<Arc<dyn TaskDao>>,
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
    let updated = tasks
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

    let row = tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    if let Some(ref events) = events {
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

    let resp = task_presenter
        .enrich_task(&row, &auth.user_id.to_string())
        .await?;
    Ok(Json(resp))
}

/// Deactivate all of a task's external bindings (best-effort).
///
/// Called when a task is closed or deleted. A closed task is terminal, so its
/// adapter "listeners" (Linear issue / GitHub PR / Slack thread bindings) must
/// stop routing inbound events — otherwise a new comment would re-provision the
/// sandbox and resurrect the task. Logs on failure but never blocks the
/// close/delete; the runtime guards in `SandboxWaker::wake` and
/// `send_message_flow` are the backstop if this doesn't run.
pub(crate) async fn deactivate_task_bindings(bindings: &dyn TaskBindings, task_id: &str) {
    match bindings.deactivate_for_task(task_id).await {
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_task(
    State(sandbox): State<Arc<dyn SandboxLifecycle>>,
    State(task_bindings): State<Arc<dyn TaskBindings>>,
    State(tasks): State<Arc<dyn TaskDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    let scope_ctx = auth.scope_context();

    // Leave the active member set before lifecycle decides whether this is a
    // member detach or the final sandbox-scoped remove. Concurrent deletes of
    // all members then serialize with at least one caller seeing no open/error
    // peers, so runtime resources are removed before the last row disappears.
    let closed = tasks
        .update_task(
            &task_id,
            &TaskPatch {
                status: Some(TaskStatus::Closed),
                ..Default::default()
            },
            &scope_ctx,
        )
        .await?;
    if !closed {
        return Err(TaskError::NotFound(task_id.clone()));
    }

    // Stop external listeners. `task_bindings.task_id` is not an FK, so rows
    // don't cascade on delete — deactivate them so inbound routing stops.
    deactivate_task_bindings(task_bindings.as_ref(), &task_id).await;

    // Permanent deletion is strict remove with archive deletion. The row is
    // deleted only on success; on failure it survives Closed with lifecycle
    // intent for the reconciler to finish.
    sandbox
        .remove(&task_id, ArchiveRetention::DeleteArchives)
        .await?;

    let deleted = tasks.delete_task(&task_id, &scope_ctx).await?;
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn mark_task_read(
    State(tasks): State<Arc<dyn TaskDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    // Ensure the task exists and is visible to the user before recording a read.
    tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    tasks
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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn follow_task(
    State(tasks): State<Arc<dyn TaskDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<FollowTaskResponse>, TaskError> {
    // Ensure the task exists and is visible to the user before following.
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
    State(tasks): State<Arc<dyn TaskDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<FollowTaskResponse>, TaskError> {
    // Ensure the task exists and is visible to the user before unfollowing.
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
    State(tasks): State<Arc<dyn TaskDao>>,
    State(task_relations): State<Arc<dyn TaskRelationDao>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    req: Option<Json<CloseTaskRequest>>,
) -> Result<Json<TaskResponse>, TaskError> {
    let req = req.map(|Json(req)| req).unwrap_or_default();
    let scope_ctx = auth.scope_context();
    // Read-check + control-check: find_by_id uses access_filter (visibility);
    // the inline can_write check enforces owner/admin policy without the
    // deleted ensure_can_write_task helper.
    let row = tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    if !scope_ctx.can_write(&task_resource(&row)) {
        return Err(TaskError::NotFound(task_id.clone())); // existence hiding
    }

    let children = task_relations
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
        task_closer.close_tree(&task_id, &mut visited).await?;
    } else {
        task_closer
            .close_single(&task_id, &CloseEvent::neutral())
            .await?;
    }
    tracing::info!(task_id = %task_id, user_id = %auth.user_id, "Task closed");

    let updated = tasks
        .find_by_id(&task_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = task_presenter
        .enrich_task(&updated, &auth.user_id.to_string())
        .await?;
    Ok(Json(resp))
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
        (status = 400, description = "Task is not closed", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn reopen_task(
    State(tasks): State<Arc<dyn TaskDao>>,
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

/// Reopen one closed task on `scope_ctx`'s behalf.
///
/// The status check is what makes this safe to offer as an undo: a task that
/// was never closed is refused rather than quietly rewritten.
pub async fn reopen_task_for(
    tasks: &dyn TaskDao,
    events: Option<&dyn EventPublisher>,
    scope_ctx: &ScopeContext,
    task_id: &str,
) -> Result<(), TaskError> {
    let row = tasks
        .find_by_id(task_id, scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
    if !scope_ctx.can_write(&task_resource(&row)) {
        return Err(TaskError::NotFound(task_id.to_string())); // existence hiding
    }

    if row.status != TaskStatus::Closed {
        return Err(TaskError::BadRequest(format!(
            "Cannot reopen task in '{}' status (only closed tasks can be reopened)",
            row.status
        )));
    }

    let updated = tasks
        .update_task(
            task_id,
            &TaskPatch {
                status: Some(TaskStatus::Open),
                ..Default::default()
            },
            scope_ctx,
        )
        .await?;
    if !updated {
        return Err(TaskError::NotFound(task_id.to_string()));
    }

    tracing::info!(task_id, user_id = %scope_ctx.user_id, "Task reopened");

    if let Some(events) = events {
        events
            .publish_task_event(
                task_id,
                TaskSubType::Open,
                Some(row.agent_type.as_str()),
                serde_json::json!({"reason": "reopened"}),
            )
            .await;
    }

    Ok(())
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
        (status = 400, description = "Invalid task state", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found, or the caller has no control access to it", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "six ports: starting a task touches the task row, its repository, preflight, events, the sandbox and git-sync; each is read here, none forwarded"
)]
pub async fn start_task(
    State(tasks): State<Arc<dyn TaskDao>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    State(preflight): State<Arc<dyn PreflightPort>>,
    State(events): State<Option<Arc<dyn EventPublisher>>>,
    State(sandbox): State<Arc<dyn SandboxLifecycle>>,
    State(git_sync): State<Arc<GitSyncService>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<axum::http::StatusCode, TaskError> {
    let scope_ctx = auth.scope_context();
    let row = tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    if !scope_ctx.can_control(&task_resource(&row), row.visibility) {
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
    let repo = repositories
        .describe(&auth.scope_context(), &row.repository_id)
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(row.repository_id.clone()))?;

    if let Err(e) = preflight.validate(&row, &repo).await {
        tracing::warn!(task_id = %task_id, error = %e, "Start preflight failed");
        if let Err(db_err) = tasks
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
        if let Some(ref events) = events {
            events
                .publish_sandbox_event_with_payload(
                    &task_id,
                    SandboxStatus::Error,
                    &SandboxEventPayload {
                        message: e.user_message.clone(),
                        error: Some(e.detail.clone()),
                        lifecycle_event: Some("preflight_failed".to_string()),
                        ..Default::default()
                    },
                )
                .await;
        }
        return Err(TaskError::BadRequest(e.user_message));
    }

    // Best-effort: if the repo ships a `.moatless/` directory, reconcile its
    // workspace config before provisioning. Loops are never synchronized as a
    // side effect of task start.
    git_sync.sync_repo_config_on_start(&auth, &repo).await;

    sandbox.start(&task_id).await?;

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
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn stop_task(
    State(tasks): State<Arc<dyn TaskDao>>,
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
    State(tasks): State<Arc<dyn TaskDao>>,
    State(tags_dao): State<Arc<dyn TagDao>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SetTagsRequest>,
) -> Result<Json<Vec<TagResponse>>, TaskError> {
    let scope_ctx = auth.scope_context();
    let existing = tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    if !scope_ctx.can_write(&task_resource(&existing)) {
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
    State(tasks): State<Arc<dyn TaskDao>>,
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
        (status = 400, description = "Invalid TTL value", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn update_ttl(
    State(tasks): State<Arc<dyn TaskDao>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
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
    let updated = tasks
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

    let row = tasks
        .find_by_id(&task_id, &scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let resp = task_presenter
        .enrich_task(&row, &auth.user_id.to_string())
        .await?;
    Ok(Json(resp))
}

// =========================================================================
// Helpers
// =========================================================================

pub(crate) fn task_resource(row: &Task) -> ScopedResource {
    row.scoped_resource()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::task::dao::MockTaskDao;
    use crate::task::models::{SandboxDesiredState, SandboxStopReason};
    use moatless_backend_core::store::StoreError;
    use uuid::Uuid;

    struct RecordingSandbox {
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait::async_trait]
    impl SandboxLifecycle for RecordingSandbox {
        async fn start(
            &self,
            _task_id: &str,
        ) -> Result<(), moatless_backend_core::sandbox::SandboxOpError> {
            unimplemented!("not reachable from delete_task ordering test")
        }

        async fn stop(
            &self,
            _task_id: &str,
            _reason: SandboxStopReason,
        ) -> Result<(), moatless_backend_core::sandbox::SandboxOpError> {
            unimplemented!("not reachable from delete_task ordering test")
        }

        async fn remove(
            &self,
            _task_id: &str,
            _retention: ArchiveRetention,
        ) -> Result<(), moatless_backend_core::sandbox::SandboxOpError> {
            self.log.lock().expect("log").push("remove");
            Ok(())
        }

        async fn archive_storage(
            &self,
            _task_id: &str,
        ) -> Result<(), moatless_backend_core::sandbox::SandboxOpError> {
            unimplemented!("not reachable from delete_task ordering test")
        }

        async fn stopped_with_storage(
            &self,
            _task_ids: &[String],
        ) -> Result<
            std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
            moatless_backend_core::sandbox::SandboxOpError,
        > {
            unimplemented!("not reachable from delete_task ordering test")
        }

        async fn desired_state(
            &self,
            _task_id: &str,
        ) -> Result<SandboxDesiredState, moatless_backend_core::sandbox::SandboxOpError> {
            unimplemented!("not reachable from delete_task ordering test")
        }
    }

    /// `TaskBindings` fake that only answers `deactivate_for_task`.
    ///
    /// It records the task ids it was asked to deactivate and replays a queued
    /// outcome, which is everything [`deactivate_task_bindings`] can observe.
    /// The remaining port methods are unreachable from this seam and panic
    /// rather than pretending to have behaviour no test asserts.
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
            }) as Arc<dyn SandboxLifecycle>),
            State(Arc::new(FakeBindings::returning(Ok(0))) as Arc<dyn TaskBindings>),
            State(Arc::new(tasks) as Arc<dyn TaskDao>),
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
}
