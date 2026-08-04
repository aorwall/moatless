//! Creating a task, as a call rather than as a route.
//!
//! Everything a task creation *means* lives here: which repository the task runs
//! against, whether the caller may use it, what scope the task is owned under,
//! and the credential check that would otherwise surface minutes later as a
//! sandbox dying on sign-in.
//!
//! [`TaskCreationService::create`](super::creation::TaskCreationService::create)
//! below this is the mechanics — short id, preflight, insert, relations,
//! provision, first message, event. It takes an already-normalised request and
//! resolves nothing.
//!
//! This split exists because the T3 compatibility surface creates tasks from a
//! WebSocket frame, where there is no HTTP body to parse and no HTTP response to
//! shape, and duplicating the policy above into that surface would have meant
//! two places to keep a credential check and an access check correct.

use std::future::Future;

use moatless_events::TaskSubType;

use super::creation::{FirstMessage, NewTaskRequest, TaskCreationOutcome};
use super::error::TaskError;
use super::models::Task;
use super::relations::CreatedByParent;
use super::schemas::CreateTaskRequest;
use crate::access_control::{ensure_can_use_repository, resolve_scope_for_create};
use crate::agent::catalog;
use crate::repository::models::RepositoryKind;
use crate::shared_models::AgentType;
use crate::state::AppState;
use crate::task::models::Visibility;
use moatless_backend_core::scope::{Scope, ScopeContext, ScopedResource};

/// A create request plus the two things only an internal caller supplies.
pub struct CreateTaskIntent {
    pub request: CreateTaskRequest,
    /// The task's id when the caller owns it — see
    /// [`NewTaskRequest::task_id`].
    pub task_id: Option<String>,
    /// Whether to start a sandbox as part of creation.
    pub provision: bool,
    /// The first message's uuid when the caller owns it — see
    /// [`crate::message::send::MessageSendService::send_message_flow`].
    pub first_message_uuid: Option<String>,
}

impl CreateTaskIntent {
    /// The ordinary case: the server picks every id and starts a sandbox.
    pub fn new(request: CreateTaskRequest) -> Self {
        Self {
            request,
            task_id: None,
            provision: true,
            first_message_uuid: None,
        }
    }
}

/// Create a task for `user_id`, resolving repository, access and scope first.
///
/// `build_event_payload` is the caller's, because what belongs in a task event
/// differs by caller and the HTTP path enriches the row for it. The first
/// message, when there is one, is always delivered the same way and is this
/// function's own.
pub async fn create_task<EvFut>(
    state: &AppState,
    scope_ctx: &ScopeContext,
    user_id: &str,
    intent: CreateTaskIntent,
    build_event_payload: impl FnOnce(Task) -> EvFut + Send,
) -> Result<TaskCreationOutcome, TaskError>
where
    EvFut: Future<Output = serde_json::Value> + Send,
{
    let new_task = normalize(state, scope_ctx, user_id, intent).await?;

    let state_msg = state.clone();
    let outcome = state
        .task_creation
        .create(new_task, build_event_payload, move |task_id, fm| async move {
            if let Err(e) = state_msg.message_send.send_message_flow(
                &task_id,
                &fm.text,
                fm.model.as_deref(),
                fm.agent_mode.as_ref(),
                fm.effort.as_deref(),
                &fm.file_ids,
                &fm.contexts,
                true,
                crate::message::send::ExternalOrigin::none(),
                false,
                fm.sent_by.as_deref(),
                fm.message_uuid,
            )
            .await
            {
                tracing::warn!(task_id = %task_id, error = %e, "Failed to send initial message");
            }
        })
        .await?;

    Ok(outcome)
}

/// Stop whatever a task's agent is doing.
///
/// Lifecycle owns the `Stopped` intent, stops compute and issues the graceful
/// `StopAgent`. The event that follows is what flips task status back to open
/// and fans the stop out; its idempotency guard dedupes this against the
/// `agent.stopped` the sandbox publishes in response, so the stop still lands
/// cleanly when the sandbox is already gone.
pub async fn stop_task(
    state: &AppState,
    scope_ctx: &ScopeContext,
    task_id: &str,
) -> Result<(), TaskError> {
    let task = state
        .tasks
        .find_by_id(task_id, scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
    if !scope_ctx.can_control(&ScopedResource::borrowed(
        task.scope,
        task.owner_user_id.as_deref().or(task.created_by.as_deref()),
    )) {
        // Existence hiding: a task the caller may not control is reported the
        // same way as one that is not there.
        return Err(TaskError::NotFound(task_id.to_string()));
    }

    state
        .sandbox
        .stop(task_id, crate::task::models::SandboxStopReason::User)
        .await?;

    if let Some(ref events) = state.events {
        events
            .publish_agent_event(
                task_id,
                moatless_events::AgentSubType::Stopped,
                None,
                serde_json::json!({
                    "source": crate::task::models::SandboxStopReason::User.as_str()
                }),
            )
            .await;
    }

    tracing::info!(task_id = %task_id, "Sandbox stop requested");
    Ok(())
}

/// Turn a create request into the normalised form the creation service takes.
///
/// Kept separate so the resolution rules are readable on their own and so a
/// caller that needs the resolved request without creating anything can reach
/// them.
async fn normalize(
    state: &AppState,
    scope_ctx: &ScopeContext,
    user_id: &str,
    intent: CreateTaskIntent,
) -> Result<NewTaskRequest, TaskError> {
    let CreateTaskIntent {
        request: req,
        task_id,
        provision,
        first_message_uuid,
    } = intent;
    let agent_type = validate_agent_type(req.agent_type)?;

    // A task accepts `repository_id` and/or `workspace_id`. When a workspace is
    // supplied it drives the task's repositories (workspace wins): the effective
    // repository is the workspace's primary placement. When only `repository_id`
    // is given, the single-repo path is unchanged and the shadow workspace is
    // derived downstream. At least one of the two is required.
    let workspace = match req.workspace_id.as_deref() {
        Some(workspace_id) => {
            let workspace = state
                .workspaces
                .find_by_id(workspace_id, scope_ctx)
                .await?
                .ok_or_else(|| TaskError::NotFound(format!("workspace {workspace_id}")))?;
            if workspace.deleted == Some(true) {
                return Err(TaskError::BadRequest(format!(
                    "Workspace {workspace_id} has been deleted; restore it from Git before creating tasks"
                )));
            }
            Some(workspace)
        }
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
        .find_by_id(&repository_id, scope_ctx)
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(repository_id.clone()))?;
    ensure_can_use_repository(
        scope_ctx,
        &ScopedResource::borrowed(
            repo.scope,
            repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
        ),
    )?;

    if let Some(ref name) = req.name {
        validate_task_name(name)?;
    }

    let is_template = repo.kind == RepositoryKind::Template;

    // Resolve scope for the new task. Repositories are always user- or
    // global-scoped now; tasks always inherit User scope owned by the creator.
    // `resolve_scope_for_create` is sync and pure: no DB or team lookup needed.
    let resolved_scope = resolve_scope_for_create(scope_ctx, Some(Scope::User))
        .map_err(|e| TaskError::Forbidden(e.to_string()))?;
    let (task_scope, task_owner_user_id) = (resolved_scope.scope, resolved_scope.owner_user_id);

    // Fast-fail codex tasks with no usable credential: a missing connection
    // or permanent refresh failure would otherwise surface minutes later as
    // a sandbox that dies on sign-in. Transient refresh errors (upstream
    // outage) do NOT block creation — the sandbox retries with the stored
    // blob.
    if agent_type.as_str() == "codex" {
        if let Err(crate::credentials::CredentialError::NotFound(reason)) = state
            .credentials_svc
            .ensure_fresh_codex_for(user_id, None)
            .await
        {
            tracing::info!(
                user_id = %user_id,
                reason = %reason,
                "Rejecting codex task creation: no usable Codex credential"
            );
            return Err(TaskError::BadRequest(
                "Your Codex connection is missing or has expired. Reconnect Codex in Settings before creating a Codex task.".to_string(),
            ));
        }
    }

    Ok(NewTaskRequest {
        repository_id: repository_id.clone(),
        agent_type,
        name: req.name.clone(),
        description: req.description.clone(),
        branch: req.branch.clone().or(repo.default_branch.clone()),
        model: req.model.clone(),
        created_by: Some(user_id.to_string()),
        headless: req.headless.unwrap_or(false),
        visibility: Visibility::Private,
        scope: task_scope,
        owner_user_id: task_owner_user_id,
        loop_id: None,
        workspace_id: req.workspace_id.clone(),
        template_id: if is_template {
            Some(repository_id)
        } else {
            None
        },
        vibe: if is_template { Some(true) } else { None },
        append_system_prompt: None,
        ttl_minutes: state.config.sandbox.idle_timeout_minutes as i32,
        inactive_ttl_days: state.config.sandbox.inactive_task_timeout_days as i32,
        tag_ids: vec![],
        parent: req.parent_task_id.as_deref().map(|p| CreatedByParent {
            parent_task_id: p.to_string(),
            context: serde_json::json!({ "via": "create_task_request" }),
        }),
        related_to: None,
        first_message: req.message.clone().map(|text| FirstMessage {
            text,
            model: req.model.clone(),
            agent_mode: req.agent_mode.clone(),
            effort: req.effort.clone(),
            file_ids: req.file_ids.clone(),
            contexts: req.contexts.clone(),
            sent_by: Some(user_id.to_string()),
            message_uuid: first_message_uuid,
        }),
        task_id,
        provision,
        event_subtype: TaskSubType::Created,
        // Reuse the repo already fetched for access control so the service
        // skips a redundant lookup.
        repository: Some(repo),
    })
}

pub(super) fn validate_task_name(name: &str) -> Result<(), TaskError> {
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

fn validate_agent_type(agent_type: Option<AgentType>) -> Result<AgentType, TaskError> {
    let Some(agent_type) = agent_type else {
        return Err(agent_type_error("agentType is required"));
    };
    if catalog::find(agent_type.as_str()).is_none() {
        return Err(agent_type_error(&format!(
            "unknown agentType `{}`",
            agent_type.as_str()
        )));
    }
    Ok(agent_type)
}

fn agent_type_error(reason: &str) -> TaskError {
    let options = catalog::catalog()
        .iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>()
        .join(", ");
    TaskError::BadRequest(format!("{reason}. Available options: {options}"))
}
