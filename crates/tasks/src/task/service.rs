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
use std::sync::Arc;

use moatless_types::task::TaskSubType;

use super::creation::{FirstMessage, NewTaskRequest, TaskCreationOutcome, TaskCreationService};
use super::dao::TaskDao;
use super::error::TaskError;
use super::models::Task;
use super::relations::CreatedByParent;
use super::schemas::CreateTaskRequest;
use crate::agent::catalog;
use crate::credential_sync::CredentialSync;
use crate::message::send::MessageSendService;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_ports::identity::UserResolver;
use moatless_ports::sandbox::SandboxLifecycle;
use moatless_ports::workspace::{RepositoryReader, WorkspaceResolver};
use moatless_types::access_control::ensure_can_use_repository;
use moatless_types::access_control::{ScopeContext, ScopedResource};
use moatless_types::agent::AgentType;
use moatless_types::repository::RepositoryKind;
use moatless_types::task::Visibility;

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

/// Create a task from an already-[`normalize`]d request.
///
/// [`normalize`] resolves the repository, access and scope; this runs the
/// creation and delivers the first message. The two are split so each names
/// only the ports it reads — `normalize` the four resolution DAOs, this the
/// `task_creation` service and `message_send` — and every caller runs them in
/// order.
///
/// `build_event_payload` is the caller's, because what belongs in a task event
/// differs by caller and the HTTP path enriches the row for it. The first
/// message, when there is one, is always delivered the same way and is this
/// function's own.
pub async fn create_task<EvFut>(
    task_creation: &TaskCreationService,
    message_send: Arc<MessageSendService>,
    new_task: NewTaskRequest,
    build_event_payload: impl FnOnce(Task) -> EvFut + Send,
) -> Result<TaskCreationOutcome, TaskError>
where
    EvFut: Future<Output = serde_json::Value> + Send,
{
    let outcome = task_creation
        .create(new_task, build_event_payload, move |task_id, fm| async move {
            if let Err(e) = message_send.send_message_flow(
                &task_id,
                &fm.text,
                fm.model.as_deref(),
                fm.agent_mode.as_ref(),
                fm.effort.as_deref(),
                &fm.file_ids,
                &fm.contexts,
                true,
                moatless_types::task::ExternalOrigin::none(),
                false,
                fm.sent_by.as_deref(),
                &fm.skills,
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
    tasks: &dyn TaskDao,
    sandbox: &dyn SandboxLifecycle,
    events: Option<&dyn EventPublisher>,
    scope_ctx: &ScopeContext,
    task_id: &str,
) -> Result<(), TaskError> {
    require_control(tasks, scope_ctx, task_id).await?;

    sandbox
        .stop(task_id, crate::task::models::SandboxStopReason::User)
        .await?;

    publish_user_stop(events, task_id).await;

    tracing::info!(task_id = %task_id, "Sandbox stop requested");
    Ok(())
}

/// Stop the agent mid-turn, leaving the task's sandbox running.
///
/// The narrower half of [`stop_task`]: the same graceful `StopAgent` and the
/// same terminal event, without the `Stopped` intent — so the sandbox, its
/// working copy and its preview servers stay up and the next turn runs in
/// place. The stop lands through the event either way, so a sandbox that
/// cannot be reached still closes the turn.
pub async fn stop_agent(
    tasks: &dyn TaskDao,
    sandbox: &dyn SandboxLifecycle,
    events: Option<&dyn EventPublisher>,
    scope_ctx: &ScopeContext,
    task_id: &str,
) -> Result<(), TaskError> {
    require_control(tasks, scope_ctx, task_id).await?;

    sandbox
        .stop_agent(task_id, crate::task::models::SandboxStopReason::User)
        .await;

    publish_user_stop(events, task_id).await;

    tracing::info!(task_id = %task_id, "Agent stop requested");
    Ok(())
}

/// Existence hiding: a task the caller may not control is reported the same way
/// as one that is not there.
async fn require_control(
    tasks: &dyn TaskDao,
    scope_ctx: &ScopeContext,
    task_id: &str,
) -> Result<(), TaskError> {
    let task = tasks
        .find_by_id(task_id, scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
    if !scope_ctx.can_control(
        &ScopedResource::borrowed(
            task.scope,
            task.owner_user_id.as_deref().or(task.created_by.as_deref()),
        ),
        task.visibility,
    ) {
        return Err(TaskError::NotFound(task_id.to_string()));
    }
    Ok(())
}

async fn publish_user_stop(events: Option<&dyn EventPublisher>, task_id: &str) {
    if let Some(events) = events {
        events
            .publish_agent_event(
                task_id,
                moatless_types::agent::AgentSubType::Stopped,
                None,
                serde_json::json!({
                    "source": crate::task::models::SandboxStopReason::User.as_str()
                }),
            )
            .await;
    }
}

/// What [`normalize`] resolves a create request against.
///
/// Grouped because every caller supplies all five together and none of them
/// varies independently of the rest.
pub struct CreateTaskPorts<'a> {
    pub workspaces: &'a dyn WorkspaceResolver,
    pub repositories: &'a dyn RepositoryReader,
    pub users: &'a dyn UserResolver,
    pub credential_sync: &'a CredentialSync,
    pub config: &'a Config,
}

/// Turn a create request into the normalised form the creation service takes.
///
/// The repository, access and scope resolution lives here, apart from
/// [`create_task`], so each caller runs it as the first of the two create
/// calls and hands the [`NewTaskRequest`] on. A caller that needs the resolved
/// request without creating anything stops here.
pub async fn normalize(
    ports: CreateTaskPorts<'_>,
    scope_ctx: &ScopeContext,
    user_id: &str,
    intent: CreateTaskIntent,
) -> Result<NewTaskRequest, TaskError> {
    let CreateTaskPorts {
        workspaces,
        repositories,
        users,
        credential_sync,
        config,
    } = ports;
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
            let workspace = workspaces
                .describe(scope_ctx, workspace_id)
                .await?
                .ok_or_else(|| TaskError::NotFound(format!("workspace {workspace_id}")))?;
            if workspace.deleted {
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
            workspaces
                .placements(&ws.id)
                .await?
                .into_iter()
                .next()
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
    let repo = repositories
        .describe(scope_ctx, &repository_id)
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

    let run_as_user_id = resolve_run_as(users, scope_ctx, req.run_as_user_id.as_deref()).await?;

    let is_template = repo.kind == RepositoryKind::Template;

    // Fast-fail codex tasks with no usable credential: a missing connection
    // or permanent refresh failure would otherwise surface minutes later as
    // a sandbox that dies on sign-in. Transient refresh errors (upstream
    // outage) do NOT block creation — the sandbox retries with the stored
    // blob.
    if agent_type.as_str() == "codex" {
        if let Err(moatless_workspaces::credentials::CredentialError::NotFound(reason)) =
            credential_sync.ensure_fresh_codex_for(user_id, None).await
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
        run_as_user_id,
        headless: req.headless.unwrap_or(false),
        visibility: Visibility::Private,
        loop_id: None,
        workspace_id: req.workspace_id.clone(),
        sandbox_id: req.sandbox_id.clone(),
        checkout_mode: req.checkout_mode,
        template_id: if is_template {
            Some(repository_id)
        } else {
            None
        },
        vibe: if is_template { Some(true) } else { None },
        append_system_prompt: None,
        ttl_minutes: config.sandbox.idle_timeout_minutes as i32,
        inactive_ttl_days: config.sandbox.inactive_task_timeout_days as i32,
        tag_ids: vec![],
        skills: req.skills.clone(),
        parent: req.parent_task_id.as_deref().map(|p| CreatedByParent {
            parent_task_id: p.to_string(),
            context: serde_json::json!({ "via": "create_task_request" }),
        }),
        related_to: None,
        // A create request never forks: the fork handler builds its own
        // `NewTaskRequest` on top of what this returns.
        forked_from: None,
        // Skills alone are enough to start the agent: they frontload into the
        // first turn on the pod. So a first message is delivered whenever there is
        // text or a skill to activate — an empty prompt with skills still opens a
        // turn, whereas a request with neither leaves the task idle as before.
        first_message: if req.message.is_some() || !req.skills.is_empty() {
            Some(FirstMessage {
                text: req.message.clone().unwrap_or_default(),
                model: req.model.clone(),
                agent_mode: req.agent_mode.clone(),
                effort: req.effort.clone(),
                file_ids: req.file_ids.clone(),
                contexts: req.contexts.clone(),
                sent_by: Some(user_id.to_string()),
                message_uuid: first_message_uuid,
                skills: req.skills.clone(),
            })
        } else {
            None
        },
        task_id,
        provision,
        event_subtype: TaskSubType::Created,
        // Reuse the repo already fetched for access control so the service
        // skips a redundant lookup.
        repository: Some(repo),
    })
}

/// Resolve and authorize an explicit run-as target.
///
/// `None` in, `None` out: the task then runs as its Workspace's account or its
/// owner, unchanged. A reference that names nobody is a bad request rather than
/// a silent fallback, because falling back would run the work as the caller
/// under a name they did not ask for.
async fn resolve_run_as(
    users: &dyn UserResolver,
    scope_ctx: &ScopeContext,
    requested: Option<&str>,
) -> Result<Option<String>, TaskError> {
    let Some(requested) = requested.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    let target = users
        .run_as_target(requested)
        .await
        .map_err(|e| TaskError::Internal(e.to_string()))?
        .ok_or_else(|| TaskError::BadRequest(format!("Run-as user {requested} does not exist")))?;
    if !scope_ctx.can_run_as(&target.id.to_string(), target.is_bot) {
        return Err(TaskError::Forbidden(format!(
            "Not allowed to run as {}. An admin may run a task as a bot user; anyone else only as themselves",
            target.login
        )));
    }
    Ok(Some(target.id.to_string()))
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

#[cfg(test)]
mod tests {
    use moatless_ports::identity::{MockUserResolver, RunAsUser};
    use uuid::Uuid;

    use super::*;

    const ADMIN: &str = "11111111-1111-1111-1111-111111111111";
    const BOT: &str = "22222222-2222-2222-2222-222222222222";
    const OTHER: &str = "33333333-3333-3333-3333-333333333333";

    fn ctx(user_id: &str, is_admin: bool) -> ScopeContext {
        ScopeContext {
            user_id: user_id.to_string(),
            is_admin,
        }
    }

    fn resolving_to(id: &'static str, login: &'static str, is_bot: bool) -> MockUserResolver {
        let mut users = MockUserResolver::new();
        users.expect_run_as_target().returning(move |_| {
            Ok(Some(RunAsUser {
                id: Uuid::parse_str(id).unwrap(),
                login: login.to_string(),
                is_bot,
            }))
        });
        users
    }

    #[tokio::test]
    async fn no_run_as_asked_for_resolves_to_none() {
        let mut users = MockUserResolver::new();
        users.expect_run_as_target().never();
        // Whitespace is not a reference: the CLI passes a bare `--run-as ''`
        // through, and treating it as a name would 400 on a request that asked
        // for nothing.
        for requested in [None, Some(""), Some("   ")] {
            let resolved = resolve_run_as(&users, &ctx(ADMIN, true), requested)
                .await
                .expect("no run-as is not an error");
            assert_eq!(resolved, None, "requested {requested:?}");
        }
    }

    #[tokio::test]
    async fn an_admin_may_run_as_a_bot() {
        let users = resolving_to(BOT, "soap-reviewer[bot]", true);
        let resolved = resolve_run_as(&users, &ctx(ADMIN, true), Some("soap-reviewer[bot]"))
            .await
            .expect("admin over a bot");
        assert_eq!(resolved.as_deref(), Some(BOT));
    }

    #[tokio::test]
    async fn an_admin_may_not_borrow_a_persons_account() {
        let users = resolving_to(OTHER, "someone", false);
        let error = resolve_run_as(&users, &ctx(ADMIN, true), Some("someone"))
            .await
            .expect_err("a person is not a bot");
        assert!(matches!(error, TaskError::Forbidden(_)), "{error:?}");
    }

    #[tokio::test]
    async fn a_non_admin_may_not_run_as_a_bot() {
        let users = resolving_to(BOT, "soap-reviewer[bot]", true);
        let error = resolve_run_as(&users, &ctx(OTHER, false), Some("soap-reviewer[bot]"))
            .await
            .expect_err("running as another identity is an admin power");
        assert!(matches!(error, TaskError::Forbidden(_)), "{error:?}");
    }

    #[tokio::test]
    async fn naming_yourself_is_allowed_and_needs_no_admin() {
        let users = resolving_to(OTHER, "someone", false);
        let resolved = resolve_run_as(&users, &ctx(OTHER, false), Some("someone"))
            .await
            .expect("acting as yourself changes nothing");
        assert_eq!(resolved.as_deref(), Some(OTHER));
    }

    /// A reference nobody matches is a bad request. Falling back to the caller
    /// would run the work under a name they did not ask for.
    #[tokio::test]
    async fn an_unknown_run_as_reference_is_rejected() {
        let mut users = MockUserResolver::new();
        users.expect_run_as_target().returning(|_| Ok(None));
        let error = resolve_run_as(&users, &ctx(ADMIN, true), Some("nobody"))
            .await
            .expect_err("unknown reference");
        assert!(matches!(error, TaskError::BadRequest(_)), "{error:?}");
    }
}
