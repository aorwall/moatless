//! Creating a task, as a call rather than as a route.
//!
//! Everything a task creation *means* lives here: which repository it runs
//! against, whether the caller may use it, what scope it's owned under, and the
//! credential check that would otherwise surface later as a sandbox dying on
//! sign-in.
//!
//! [`TaskCreationService::create`](super::creation::TaskCreationService::create)
//! is the mechanics below this — short id, preflight, insert, provision, event.

use std::future::Future;
use std::sync::Arc;

use moatless_types::task::TaskSubType;

use super::creation::{FirstMessage, NewTaskRequest, TaskCreationOutcome, TaskCreationService};
use super::error::TaskError;
use super::models::{Task, TaskStatus};
use super::operations::TaskCommands;
use super::relations::CreatedByParent;
use super::schemas::CreateTaskRequest;
use crate::agent::catalog;
use crate::credential_sync::CredentialSync;
use crate::message::send::MessageSendService;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_ports::identity::UserResolver;
use moatless_ports::sandbox::SandboxLifecycle;
use moatless_types::access_control::ensure_can_use_repository;
use moatless_types::access_control::{ScopeContext, ScopedResource};
use moatless_types::agent::AgentProfile;
use moatless_types::task::Visibility;
use moatless_workspace_ports::{RepositoryReader, WorkspaceResolver};
use moatless_workspace_types::repository::RepositoryKind;

/// A create request plus the two things only an internal caller supplies.
pub struct CreateTaskIntent {
    pub request: CreateTaskRequest,
    /// The task's id when the caller owns it — see [`NewTaskRequest::task_id`].
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

/// Create a task from an already-[`normalize`]d request. [`normalize`] resolves
/// repository, access and scope; this runs creation and delivers the first
/// message via `build_event_payload`.
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
        .create(
            new_task,
            build_event_payload,
            move |task_id, fm| async move {
                message_send
                    .send_message_flow(
                        &task_id,
                        &fm.text,
                        fm.model.as_deref(),
                        fm.agent_mode.as_ref(),
                        fm.effort.as_deref(),
                        &fm.file_ids,
                        &fm.contexts,
                        true,
                        moatless_types::task::ExternalOrigin {
                            source_task_id: fm.source_task_id.as_deref(),
                            ..moatless_types::task::ExternalOrigin::none()
                        },
                        false,
                        fm.sent_by.as_deref(),
                        &fm.skills,
                        fm.message_uuid,
                        false,
                        None,
                    )
                    .await
                    .map(|_| ())
            },
        )
        .await?;

    Ok(outcome)
}

/// Stop whatever a task's agent is doing. Lifecycle stops compute and issues
/// the graceful `StopAgent`; the following event flips status back to open and
/// dedupes against the sandbox's own `agent.stopped`.
pub async fn stop_task(
    tasks: &TaskCommands,
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

/// Stop the agent mid-turn, leaving the sandbox running. The narrower half of
/// [`stop_task`]: same `StopAgent` and terminal event, without the `Stopped`
/// intent, so running Scripts stay up.
pub async fn stop_agent(
    tasks: &TaskCommands,
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
    tasks: &TaskCommands,
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

pub(crate) async fn publish_user_stop(events: Option<&dyn EventPublisher>, task_id: &str) {
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

/// What [`normalize`] resolves a create request against. Grouped because every
/// caller supplies all six together and none varies independently of the rest.
pub struct CreateTaskPorts<'a> {
    pub workspaces: &'a dyn WorkspaceResolver,
    pub repositories: &'a dyn RepositoryReader,
    pub tasks: &'a TaskCommands,
    pub users: &'a dyn UserResolver,
    pub credential_sync: &'a CredentialSync,
    pub config: &'a Config,
}

/// The repository a Task in this Workspace runs against, whatever repository
/// the caller also named: the Workspace's primary placement. `None` when the
/// Workspace places none, and the Task then runs with no repository.
pub(crate) async fn primary_repository_id(
    workspaces: &dyn WorkspaceResolver,
    workspace_id: &str,
) -> Result<Option<String>, moatless_types::store::StoreError> {
    Ok(workspaces
        .placements(workspace_id)
        .await?
        .into_iter()
        .next()
        .map(|placement| placement.repository_id))
}

/// Turn a create request into the normalised form the creation service takes.
/// Repository, access and scope resolution lives here, apart from
/// [`create_task`]; a caller needing only that stops here.
pub async fn normalize(
    ports: CreateTaskPorts<'_>,
    scope_ctx: &ScopeContext,
    user_id: &str,
    intent: CreateTaskIntent,
) -> Result<NewTaskRequest, TaskError> {
    let CreateTaskPorts {
        workspaces,
        repositories,
        tasks,
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
    let agent_profile = validate_agent_profile(req.agent_profile.as_deref(), req.model.as_deref())?;

    let workspace_id = resolve_workspace_id(tasks, scope_ctx, &req).await?;
    let workspace = workspaces
        .describe(scope_ctx, &workspace_id)
        .await?
        .ok_or_else(|| TaskError::NotFound(format!("workspace {workspace_id}")))?;
    if workspace.deleted {
        return Err(TaskError::BadRequest(format!(
            "Workspace {workspace_id} has been deleted; restore it from Git before creating tasks"
        )));
    }

    // A Workspace that places no repository runs its Task in an empty
    // workspace directory, so there is nothing to check access to.
    let repository_id = primary_repository_id(workspaces, &workspace.id).await?;
    let repo = match &repository_id {
        Some(repository_id) => Some(
            repositories
                .describe(scope_ctx, repository_id)
                .await?
                .ok_or_else(|| TaskError::RepoNotFound(repository_id.clone()))?,
        ),
        None => None,
    };
    if let Some(repo) = &repo {
        ensure_can_use_repository(
            scope_ctx,
            &ScopedResource::borrowed(
                repo.scope,
                repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
            ),
        )?;
    }

    if let Some(ref name) = req.name {
        validate_task_name(name)?;
    }

    let run_as_user_id = resolve_run_as(users, scope_ctx, req.run_as_user_id.as_deref()).await?;

    let is_template = repo
        .as_ref()
        .is_some_and(|repo| repo.kind == RepositoryKind::Template);

    // Fast-fail codex tasks with no usable credential: a missing connection
    // or permanent refresh failure would otherwise surface minutes later as
    // a sandbox that dies on sign-in. Transient refresh errors (upstream
    // outage) do NOT block creation — the sandbox retries with the stored
    // blob.
    if agent_profile == AgentProfile::Codex {
        if let Err(moatless_identity::credentials::CredentialError::NotFound(reason)) =
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
        // The HTTP path has no Loop behind it, so the Sandbox takes whatever
        // its Workspace declares.
        sandbox_placement: crate::task::models::SandboxPlacement::default(),
        repository_id: repository_id.clone(),
        agent_profile,
        name: req.name.clone(),
        description: req.description.clone(),
        branch: req
            .branch
            .clone()
            .or_else(|| repo.as_ref().and_then(|repo| repo.default_branch.clone())),
        model: req.model.clone(),
        created_by: Some(user_id.to_string()),
        run_as_user_id,
        headless: req.headless.unwrap_or(false),
        visibility: Visibility::Private,
        loop_id: None,
        workspace_id,
        sandbox_id: req.sandbox_id.clone(),
        checkout_mode: req.checkout_mode,
        template_id: if is_template {
            repository_id.clone()
        } else {
            None
        },
        vibe: if is_template { Some(true) } else { None },
        append_system_prompt: None,
        ttl_minutes: config.sandbox.idle_timeout_minutes as i32,
        close_after_settled_days: config.sandbox.close_after_settled_days as i32,
        // `None` follows `TASK_SETTLE_AFTER_HOURS` as the reaper reads it.
        settle_after_hours: None,
        tag_ids: vec![],
        memory_preload: Vec::new(),
        skills: req.skills.clone(),
        parent: req.parent_task_id.as_deref().map(|p| CreatedByParent {
            parent_task_id: p.to_string(),
            context: serde_json::json!({ "via": "create_task_request" }),
        }),
        related_to: None,
        // A create request never forks: the fork handler builds its own
        // `NewTaskRequest` on top of what this returns.
        forked_from: None,
        // Skills alone can start the agent: they frontload into the first
        // turn, so a first message fires whenever there's text or a skill
        // to activate — empty prompt with skills still opens a turn, but
        // neither leaves the task idle.
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
                source_task_id: req.parent_task_id.clone(),
            })
        } else {
            None
        },
        task_id,
        provision,
        event_subtype: TaskSubType::Created,
        // Reuse the repo already fetched for access control so the service
        // skips a redundant lookup.
        repository: repo,
    })
}

/// The Workspace a create request runs in: the one it names, else its parent
/// Task's. A repository-only request is refused — a repository places many
/// Workspaces, and picking one nobody chose is wrong.
async fn resolve_workspace_id(
    tasks: &TaskCommands,
    scope_ctx: &ScopeContext,
    req: &CreateTaskRequest,
) -> Result<String, TaskError> {
    if let Some(workspace_id) = req
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|w| !w.is_empty())
    {
        return Ok(workspace_id.to_string());
    }
    let Some(parent_task_id) = req.parent_task_id.as_deref() else {
        return Err(TaskError::BadRequest("workspaceId is required".to_string()));
    };
    let parent = tasks
        .find_by_id(parent_task_id, scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(parent_task_id.to_string()))?;
    parent.workspace_id.ok_or_else(|| {
        TaskError::BadRequest(format!(
            "workspaceId is required: parent task {parent_task_id} runs in no workspace"
        ))
    })
}

/// Resolve and authorize an explicit run-as target. `None` in, `None` out — the
/// task runs as its Workspace's account or owner. A reference naming nobody is
/// a bad request, not a silent fallback.
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

/// The longest `Task.name` [`validate_task_name`] accepts, in bytes.
pub(super) const MAX_TASK_NAME_BYTES: usize = 200;

pub(super) fn validate_task_name(name: &str) -> Result<(), TaskError> {
    if name.is_empty() {
        return Err(TaskError::BadRequest(
            "Task name cannot be empty".to_string(),
        ));
    }
    if name.len() > MAX_TASK_NAME_BYTES {
        return Err(TaskError::BadRequest(format!(
            "Task name too long (max {MAX_TASK_NAME_BYTES})"
        )));
    }
    Ok(())
}

fn validate_agent_profile(
    agent_profile: Option<&str>,
    model: Option<&str>,
) -> Result<AgentProfile, TaskError> {
    let Some(agent_profile) = agent_profile else {
        return Err(agent_profile_error("agentProfile is required"));
    };
    let agent_profile = agent_profile.parse().map_err(TaskError::BadRequest)?;
    catalog::validate_agent_model(agent_profile, model).map_err(TaskError::BadRequest)?;
    Ok(agent_profile)
}

fn agent_profile_error(reason: &str) -> TaskError {
    let options = catalog::catalog()
        .map(catalog::AgentProfileEntry::id)
        .collect::<Vec<_>>()
        .join(", ");
    TaskError::BadRequest(format!("{reason}. Available options: {options}"))
}

/// Reopen a visible closed task after checking write access.
pub async fn reopen_task_for(
    tasks: &TaskCommands,
    events: Option<&dyn EventPublisher>,
    scope_ctx: &ScopeContext,
    task_id: &str,
) -> Result<(), TaskError> {
    let row = tasks
        .find_by_id(task_id, scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
    if !scope_ctx.can_write(&row.scoped_resource()) {
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
            &super::models::TaskPatch {
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
                Some(row.agent_profile.as_str()),
                serde_json::json!({"reason": "reopened"}),
            )
            .await;
    }

    Ok(())
}
/// Update supplied task expiration limits after validating their values.
pub async fn update_ttl(
    tasks: &TaskCommands,
    scope_ctx: &ScopeContext,
    task_id: String,
    req: super::schemas::UpdateTaskTtlRequest,
) -> Result<Task, TaskError> {
    if req.ttl_minutes.is_none() && req.close_after_settled_days.is_none() {
        return Err(TaskError::BadRequest(
            "provide ttl_minutes and/or close_after_settled_days".to_string(),
        ));
    }
    if req.ttl_minutes.is_some_and(|v| v < 0) {
        return Err(TaskError::BadRequest(
            "ttl_minutes must be >= 0".to_string(),
        ));
    }
    if req.close_after_settled_days.is_some_and(|v| v < 0) {
        return Err(TaskError::BadRequest(
            "close_after_settled_days must be >= 0".to_string(),
        ));
    }

    let updated = tasks
        .update_task(
            &task_id,
            &super::models::TaskPatch {
                ttl_minutes: req.ttl_minutes,
                close_after_settled_days: req.close_after_settled_days,
                ..Default::default()
            },
            scope_ctx,
        )
        .await?;
    if !updated {
        return Err(TaskError::NotFound(task_id.clone()));
    }
    tracing::info!(
        task_id = %task_id,
        user_id = %scope_ctx.user_id,
        ttl_minutes = ?req.ttl_minutes,
        close_after_settled_days = ?req.close_after_settled_days,
        "Task TTL updated"
    );

    let row = tasks
        .find_by_id(&task_id, scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    Ok(row)
}

#[cfg(test)]
mod tests {
    use moatless_ports::identity::{MockUserResolver, RunAsUser};
    use uuid::Uuid;

    use super::*;

    #[test]
    fn an_unknown_agent_profile_is_a_bad_request_listing_the_options() {
        let req: CreateTaskRequest =
            serde_json::from_str(r#"{"agentProfile":"gemini"}"#).expect("the request deserializes");
        let err = validate_agent_profile(req.agent_profile.as_deref(), None).unwrap_err();
        let TaskError::BadRequest(message) = err else {
            panic!("expected a bad request, got {err:?}");
        };
        assert!(
            message.contains("gemini") && message.contains("codex"),
            "{message}"
        );
    }

    const ADMIN: &str = "11111111-1111-1111-1111-111111111111";
    const BOT: &str = "22222222-2222-2222-2222-222222222222";
    const OTHER: &str = "33333333-3333-3333-3333-333333333333";

    fn ctx(user_id: &str, is_admin: bool) -> ScopeContext {
        ScopeContext::new(user_id.to_string(), is_admin)
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

    fn create_request(
        workspace_id: Option<&str>,
        parent_task_id: Option<&str>,
    ) -> CreateTaskRequest {
        CreateTaskRequest {
            repository_id: Some("repo-1".to_string()),
            workspace_id: workspace_id.map(str::to_string),
            sandbox_id: None,
            checkout_mode: None,
            agent_profile: None,
            name: None,
            description: None,
            branch: None,
            message: None,
            model: None,
            agent_mode: None,
            effort: None,
            contexts: Vec::new(),
            file_ids: Vec::new(),
            parent_task_id: parent_task_id.map(str::to_string),
            run_as_user_id: None,
            headless: None,
            skills: Vec::new(),
        }
    }

    fn parent_running_in(workspace_id: Option<&'static str>) -> TaskCommands {
        let mut tasks = crate::task::dao::MockTaskDao::new();
        tasks.expect_find_by_id().returning(move |task_id, _| {
            let now = chrono::Utc::now();
            Ok(Some(Task {
                task_id: task_id.to_string(),
                short_id: None,
                created_at: now,
                last_message_at: now,
                agent_profile: AgentProfile::ClaudeCode,
                repository_id: Some("repo-1".to_string()),
                workspace_id: workspace_id.map(str::to_string),
                sandbox_id: None,
                checkout_mode: None,
                status: TaskStatus::Open,
                total_cost_usd: 0.0,
                site_url: None,
                template_id: None,
                vibe: None,
                name: None,
                description: None,
                site_id: None,
                site_provider: None,
                branch: None,
                model: None,
                created_by: Some(ADMIN.to_string()),
                visibility: Visibility::Private,
                scope: moatless_types::access_control::Scope::User,
                owner_user_id: Some(ADMIN.to_string()),
                run_as_user_id: None,
                updated_at: None,
                append_system_prompt: None,
                headless: false,
                ttl_minutes: 15,
                close_after_settled_days: 7,
                settle_after_hours: None,
                loop_id: None,
                memory_preload: Vec::new(),
                skills: Vec::new(),
            }))
        });
        TaskCommands::new(Arc::new(tasks))
    }

    #[tokio::test]
    async fn a_named_workspace_is_used_as_given() {
        let tasks = TaskCommands::new(Arc::new(crate::task::dao::MockTaskDao::new()));
        let resolved = resolve_workspace_id(
            &tasks,
            &ctx(ADMIN, false),
            &create_request(Some("ws-a"), None),
        )
        .await
        .expect("a named workspace needs no lookup");
        assert_eq!(resolved, "ws-a");
    }

    #[tokio::test]
    async fn a_subtask_inherits_its_parents_workspace() {
        let tasks = parent_running_in(Some("ws-parent"));
        let resolved = resolve_workspace_id(
            &tasks,
            &ctx(ADMIN, false),
            &create_request(None, Some("task-parent")),
        )
        .await
        .expect("the parent names the workspace");
        assert_eq!(resolved, "ws-parent");
    }

    /// A repository places any number of Workspaces, so there is nothing to
    /// fall back to.
    #[tokio::test]
    async fn a_request_with_only_a_repository_is_rejected() {
        let tasks = TaskCommands::new(Arc::new(crate::task::dao::MockTaskDao::new()));
        let error = resolve_workspace_id(&tasks, &ctx(ADMIN, false), &create_request(None, None))
            .await
            .expect_err("a repository does not name a workspace");
        assert!(
            matches!(&error, TaskError::BadRequest(message) if message.contains("workspaceId is required")),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_parent_that_runs_in_no_workspace_is_rejected() {
        let tasks = parent_running_in(None);
        let error = resolve_workspace_id(
            &tasks,
            &ctx(ADMIN, false),
            &create_request(None, Some("task-parent")),
        )
        .await
        .expect_err("a parent predating workspaces names none");
        assert!(matches!(error, TaskError::BadRequest(_)), "{error:?}");
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
