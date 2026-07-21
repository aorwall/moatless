//! Sandbox handler functions.
//!
//! Extracted from routes.rs for consistency with other modules
//! (task/, message/, agent/, etc.).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use futures::stream::Stream;
use serde::Deserialize;
use tracing::{error, info, warn};

use chrono::Utc;

use super::error::SandboxError;
use super::lifecycle::{ArchiveRetention, Lifecycle, ObservedStatus};
use super::models::*;
use super::routes::SandboxApiState;
use super::schemas::*;
use crate::access_control;
use crate::extractors::OptionalScope;
use crate::task::dao::{TaskPatch, TaskRepository};
use crate::task::models::{SandboxStopReason, Task};
use crate::team::dao::TeamRepository;
use moatless_backend_core::scope::ScopeContext;

/// Provision a new sandbox for a task.
///
/// Builds the provision request internally from DB state (task, repository,
/// credentials). No request body required.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/provision",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Sandbox provisioned successfully", body = ProvisionResponse),
        (status = 500, description = "Provisioning failed", body = crate::error::ErrorBody)
    )
)]
pub async fn provision_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> Result<Json<ProvisionResponse>, SandboxError> {
    info!(task_id = %task_id, "POST /provision");
    authorize_task_control(&state, &scope, &task_id).await?;

    // Lifecycle owns plan construction and the Running intent; the bridge builds
    // the provision request from DB state internally.
    let outcome = require_lifecycle(&state)?.start(&task_id).await?;
    mark_sandbox_started(&state, &task_id).await;
    info!(task_id = %task_id, "Provision (start) succeeded");
    Ok(Json(ProvisionResponse {
        status: observed_to_sandbox_status(outcome.observed),
    }))
}

/// Get the canonical live status for a Task's Sandbox and Agent.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/live-status",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Task live status", body = crate::sandbox::live_status::LiveStatus),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn live_status_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let task = match authorize_task_read(&state, &scope, &task_id).await {
        Ok(task) => task,
        Err(e) => return e.into_response(),
    };

    match state
        .provider
        .live_status(&task_id, task.status, task.sandbox_desired_state)
        .await
    {
        Ok(status) => Json(status).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Get full sandbox status including servers and Agent status.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/status",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Full sandbox status", body = SandboxStatusFullResponse),
        (status = 503, description = "Sandbox not ready", body = crate::error::ErrorBody),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn sandbox_status_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let task = match authorize_task_read(&state, &scope, &task_id).await {
        Ok(task) => task,
        Err(e) => return e.into_response(),
    };

    // Resolve server configs from DB
    let server_configs = resolve_server_configs(&state, &task_id).await;

    match state
        .provider
        .sandbox_status(
            &task_id,
            &server_configs,
            task.status,
            task.sandbox_desired_state,
        )
        .await
    {
        Ok(mut resp) => {
            // Surface the recorded lifecycle intent and image freshness so the
            // UI can tell whether a redeploy would move the sandbox to a newer
            // image. The "latest" image is what a fresh (re)deploy would use:
            // the per-repo `docker_image` override, else the configured default.
            resp.sandbox_desired_state = Some(task.sandbox_desired_state);

            let docker_image_override = match (&state.workspaces, task.workspace_id.as_deref()) {
                (Some(workspaces), Some(ws_id)) => workspaces
                    .find_by_id(ws_id)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|ws| ws.docker_image),
                _ => None,
            };
            let latest = state
                .provider
                .expected_sandbox_image(docker_image_override.as_deref());
            resp.image_up_to_date = resp
                .current_sandbox_image
                .as_ref()
                .map(|current| current == &latest);
            resp.latest_sandbox_image = Some(latest);

            Json(resp).into_response()
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "sandbox status failed");
            e.into_response()
        }
    }
}

/// Stop a running sandbox.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/stop",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Sandbox stopped"),
        (status = 500, description = "Stop failed", body = crate::error::ErrorBody)
    )
)]
pub async fn stop_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /stop");
    authorize_task_control(&state, &scope, &task_id).await?;
    require_lifecycle(&state)?
        .stop(&task_id, SandboxStopReason::User)
        .await?;
    info!(task_id = %task_id, "Stop succeeded");
    Ok(StatusCode::NO_CONTENT)
}

/// Restart a sandbox (stop + start).
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/restart",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Sandbox restarted"),
        (status = 500, description = "Restart failed", body = crate::error::ErrorBody)
    )
)]
pub async fn restart_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /restart");
    authorize_task_control(&state, &scope, &task_id).await?;
    // lifecycle.restart detaches internally, so it runs to completion even if the
    // HTTP client disconnects.
    require_lifecycle(&state)?.restart(&task_id).await?;
    mark_sandbox_started(&state, &task_id).await;
    info!(task_id = %task_id, "Restart succeeded");
    Ok(StatusCode::NO_CONTENT)
}

/// Redeploy a sandbox (delete Deployment/Service/IngressRoutes, recreate with fresh repo config).
///
/// Preserves workspace data (PVC). Use after updating repo server configs.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/redeploy",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Sandbox redeployed successfully", body = ProvisionResponse),
        (status = 500, description = "Redeploy failed", body = crate::error::ErrorBody)
    )
)]
pub async fn redeploy_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> Result<Json<ProvisionResponse>, SandboxError> {
    info!(task_id = %task_id, "POST /redeploy");
    authorize_task_control(&state, &scope, &task_id).await?;

    // Lifecycle owns plan construction and detaches internally; the bridge
    // rebuilds the provision request from fresh DB state.
    let outcome = require_lifecycle(&state)?.redeploy(&task_id).await?;
    mark_sandbox_started(&state, &task_id).await;
    info!(task_id = %task_id, "Redeploy succeeded");
    Ok(Json(ProvisionResponse {
        status: observed_to_sandbox_status(outcome.observed),
    }))
}

/// Clean up a sandbox and all associated resources.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/cleanup",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Sandbox cleaned up"),
        (status = 500, description = "Cleanup failed", body = crate::error::ErrorBody)
    )
)]
pub async fn cleanup_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /cleanup");
    authorize_task_control(&state, &scope, &task_id).await?;
    // A cleanup deletes archives (no retention) — lifecycle owns the Removed
    // intent and strict teardown.
    require_lifecycle(&state)?
        .remove(&task_id, ArchiveRetention::DeleteArchives)
        .await?;
    info!(task_id = %task_id, "Cleanup succeeded");
    Ok(StatusCode::NO_CONTENT)
}

// =============================================================================
// New preview/workspace handlers
// =============================================================================

/// Get server status for all servers in a task's sandbox.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/server/status",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Server status", body = ServerStatusResponse),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn server_status_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = authorize_task_read(&state, &scope, &task_id).await {
        return e.into_response();
    }

    // Resolve server configs from DB
    let server_configs = resolve_server_configs(&state, &task_id).await;

    // Fetch container statuses from the provider
    let container_statuses = match state.provider.container_statuses(&task_id).await {
        Ok(cs) => cs,
        Err(e) => {
            error!(task_id = %task_id, error = %e, "server container_statuses failed");
            return e.into_response();
        }
    };

    // Merge config + container statuses into server snapshots
    let servers = merge_server_status(&server_configs, &container_statuses);
    Json(ServerStatusResponse { servers }).into_response()
}

/// Query parameters for the server logs endpoint.
#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    /// Fetch logs from the previous container instance (useful for crashed containers).
    #[serde(default)]
    pub previous: bool,
}

/// Stream server logs via SSE.
///
/// Returns a Server-Sent Events stream of log lines for the given server.
/// Pass `?previous=true` to fetch logs from a crashed/restarted container.
/// The response body cannot be represented in OpenAPI (SSE streaming).
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/servers/{server_name}/logs",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("server_name" = String, Path, description = "Server name"),
        ("previous" = bool, Query, description = "Fetch logs from previous container instance")
    ),
    responses(
        (status = 200, description = "SSE stream of server log lines"),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn server_logs_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path((task_id, server_name)): Path<(String, String)>,
    Query(query): Query<LogsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, SandboxError>>>, SandboxError> {
    authorize_task_read(&state, &scope, &task_id).await?;
    let stream = state
        .provider
        .server_logs(&task_id, &server_name, query.previous)
        .await?;
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// =============================================================================
// Authorization helpers
// =============================================================================

/// Resolve the task for `task_id` subject to the caller's scope, returning
/// `NotFound` when the caller may not read it (so we never leak the existence
/// of another tenant's task).
///
/// `scope` is `None` only in dev mode (no auth provider → middleware disabled);
/// there we fall back to the system scope, which resolves any task. In
/// production `find_by_id` bakes the caller's scope into SQL and returns `None`
/// for tasks they cannot see.
pub(super) async fn authorize_task_read(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<Task, SandboxError> {
    authorize_task_read_inner(state.tasks.as_deref(), scope, task_id).await
}

/// Like [`authorize_task_read`], but additionally requires *control* access
/// (the same policy the task HTTP API and sandbox gRPC server enforce): the
/// owner/creator of the task, a managing member of a team task, or a platform
/// admin. Read-only viewers get `Forbidden`.
pub(super) async fn authorize_task_control(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<Task, SandboxError> {
    authorize_task_control_inner(
        state.tasks.as_deref(),
        state.teams.as_deref(),
        scope,
        task_id,
    )
    .await
}

/// Core of [`authorize_task_read`] against explicit dependencies (so it can be
/// unit-tested without a `SandboxProvider`).
async fn authorize_task_read_inner(
    tasks: Option<&dyn TaskRepository>,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<Task, SandboxError> {
    let tasks = tasks.ok_or_else(|| {
        SandboxError::Internal("Task repository not available for authorization".into())
    })?;
    // Fail closed: no identity means the request never passed auth_middleware.
    let ctx = scope
        .clone()
        .ok_or_else(|| SandboxError::Forbidden("authentication required".into()))?;
    tasks
        .find_by_id(task_id, &ctx)
        .await
        .map_err(|e| SandboxError::Internal(format!("task lookup failed: {e}")))?
        .ok_or_else(|| SandboxError::NotFound(format!("task {task_id}")))
}

/// Core of [`authorize_task_control`] against explicit dependencies.
async fn authorize_task_control_inner(
    tasks: Option<&dyn TaskRepository>,
    teams: Option<&dyn TeamRepository>,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<Task, SandboxError> {
    let task = authorize_task_read_inner(tasks, scope, task_id).await?;

    let Some(ctx) = scope.as_ref() else {
        return Err(SandboxError::Forbidden("authentication required".into()));
    };

    let teams = teams.ok_or_else(|| {
        SandboxError::Internal("Team repository not available for authorization".into())
    })?;
    let allowed = access_control::can_control_task_ctx(teams, ctx, &task.scoped_resource())
        .await
        .map_err(|e| SandboxError::Internal(format!("authorization failed: {e}")))?;
    if !allowed {
        return Err(SandboxError::Forbidden(format!(
            "no control access to task {task_id}"
        )));
    }
    Ok(task)
}

// =============================================================================
// Private helpers
// =============================================================================

/// The shared lifecycle handle, or a clean "unavailable" error in the no-database
/// mode where the lifecycle handlers cannot operate.
fn require_lifecycle(state: &SandboxApiState) -> Result<&Lifecycle, SandboxError> {
    state
        .lifecycle
        .as_deref()
        .ok_or_else(|| SandboxError::Internal("Sandbox lifecycle unavailable (no database)".into()))
}

/// The `ProvisionResponse.status` a lifecycle outcome reports.
fn observed_to_sandbox_status(observed: ObservedStatus) -> moatless_events::SandboxStatus {
    use moatless_events::SandboxStatus;
    match observed {
        ObservedStatus::Ready => SandboxStatus::Ready,
        ObservedStatus::Starting => SandboxStatus::Initializing,
        ObservedStatus::Stopped => SandboxStatus::Stopped,
        ObservedStatus::Removed => SandboxStatus::Removed,
    }
}

/// Bump `sandbox_started_at` so the idle reaper doesn't reap a just-(re)started
/// sandbox whose task carries only stale message timestamps. Best-effort;
/// Sandbox intent itself is owned by the lifecycle.
async fn mark_sandbox_started(state: &SandboxApiState, task_id: &str) {
    if let Some(ref tasks) = state.tasks {
        let patch = TaskPatch {
            sandbox_started_at: Some(Utc::now()),
            ..Default::default()
        };
        if let Err(e) = tasks.update_task(task_id, &patch).await {
            warn!(task_id = %task_id, error = %e, "Failed to update sandbox_started_at");
        }
    }
}

/// Resolve server configs from the task's workspace via the SandboxApiState.
async fn resolve_server_configs(state: &SandboxApiState, task_id: &str) -> Vec<ServerConfig> {
    if let (Some(ref tasks), Some(ref workspaces)) = (&state.tasks, &state.workspaces) {
        super::auto_provision::get_server_configs_for_task(
            tasks.as_ref(),
            workspaces.as_ref(),
            task_id,
        )
        .await
    } else {
        warn!(task_id = %task_id, "No task/workspace repositories available for server config resolution");
        vec![]
    }
}

/// Merge server configs (from DB) with container statuses (from runtime)
/// into `ServerSnapshot` entries for the `ServerStatusResponse`.
fn merge_server_status(
    server_configs: &[ServerConfig],
    container_statuses: &[ContainerObservedStatus],
) -> Vec<ServerSnapshot> {
    server_configs
        .iter()
        .map(|config| {
            let container = container_statuses.iter().find(|cs| {
                let name = cs.name.strip_suffix("-server").unwrap_or(&cs.name);
                name == config.name
            });
            let status = container.map_or(
                ServerStatus::Starting,
                ContainerObservedStatus::to_server_status,
            );
            let mut snapshot = ServerSnapshot::from_config(config, status);
            snapshot.error = container.and_then(|cs| cs.waiting_reason.clone());
            snapshot
        })
        .collect()
}

#[cfg(test)]
mod authz_tests {
    //! Unit tests for the per-task sandbox authorization gate. The scoped-read
    //! SQL filter itself is covered by `scope.rs`; these exercise how the
    //! handlers compose read + control on the task a scoped read returned.
    use super::*;

    use async_trait::async_trait;
    use moatless_backend_core::scope::Scope;

    use crate::task::dao::MockTaskRepository;
    use crate::team::models::{Team, TeamMember};

    fn epoch() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(0, 0).expect("epoch is a valid timestamp")
    }

    /// Deterministic uuid for a readable test name: distinct names map to
    /// distinct ids, equal names to equal ids.
    fn uid(name: &str) -> uuid::Uuid {
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
    }

    /// Minimal `TeamRepository` stub — only the membership/role methods the
    /// control check consults are meaningful; everything else is `unimplemented!`.
    struct StubTeamRepo {
        /// `(user_id, role)` rows for `team-1`, role as stored (`owner`/`admin`/`member`).
        members: Vec<(uuid::Uuid, String)>,
    }

    impl StubTeamRepo {
        fn with_roles(members: &[(&str, &str)]) -> Self {
            Self {
                members: members
                    .iter()
                    .map(|(u, r)| (uid(u), (*r).to_string()))
                    .collect(),
            }
        }
    }

    #[async_trait]
    impl TeamRepository for StubTeamRepo {
        async fn find_members(&self, team_id: &str) -> Result<Vec<TeamMember>, sqlx::Error> {
            Ok(self
                .members
                .iter()
                .map(|(u, r)| TeamMember {
                    team_id: team_id.to_string(),
                    user_id: *u,
                    role: r.clone(),
                    created_at: epoch(),
                })
                .collect())
        }
        async fn is_member(&self, _team_id: &str, user_id: &uuid::Uuid) -> Result<bool, sqlx::Error> {
            Ok(self.members.iter().any(|(u, _)| u == user_id))
        }
        async fn find_team_ids_for_user(
            &self,
            _user_id: &uuid::Uuid,
        ) -> Result<Vec<String>, sqlx::Error> {
            Ok(vec![])
        }
        async fn find_by_id(&self, _id: &str) -> Result<Option<Team>, sqlx::Error> {
            unimplemented!()
        }
        async fn find_by_slug(&self, _slug: &str) -> Result<Option<Team>, sqlx::Error> {
            unimplemented!()
        }
        async fn find_by_external_id(
            &self,
            _external_id: &str,
            _provider: &str,
        ) -> Result<Option<Team>, sqlx::Error> {
            unimplemented!()
        }
        async fn list_all(&self) -> Result<Vec<Team>, sqlx::Error> {
            unimplemented!()
        }
        async fn insert(&self, _team: &Team) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn insert_if_absent(&self, _team: &Team) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn update(
            &self,
            _id: &str,
            _name: &str,
            _description: Option<&str>,
        ) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn delete(&self, _id: &str) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn find_teams_for_user(&self, _user_id: &uuid::Uuid) -> Result<Vec<Team>, sqlx::Error> {
            unimplemented!()
        }
        async fn add_member(
            &self,
            _team_id: &str,
            _user_id: &uuid::Uuid,
            _role: &str,
        ) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn remove_member(
            &self,
            _team_id: &str,
            _user_id: &uuid::Uuid,
        ) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn upsert_by_external(
            &self,
            _external_id: &str,
            _provider: &str,
            _name: &str,
            _slug: &str,
        ) -> Result<Team, sqlx::Error> {
            unimplemented!()
        }
    }

    /// A `Task` fixture with the given scope/owner/team; all other columns are
    /// inert defaults irrelevant to the access check.
    fn task_fixture(scope: Scope, owner_user_id: Option<&str>, team_id: Option<&str>) -> Task {
        Task {
            task_id: "task-1".to_string(),
            short_id: None,
            created_at: String::new(),
            last_message_at: String::new(),
            agent_type: crate::shared_models::AgentType("claude-code".to_string()),
            repository_id: "repo-1".to_string(),
            status: crate::task::models::TaskStatus::Open,
            total_cost_usd: 0.0,
            site_url: None,
            template_id: None,
            vibe: None,
            name: None,
            description: None,
            site_id: None,
            site_provider: None,
            branch: None,
            created_by: owner_user_id.map(|u| uid(u).to_string()),
            visibility: crate::task::models::Visibility::Private,
            scope,
            team_id: team_id.map(str::to_string),
            owner_user_id: owner_user_id.map(|u| uid(u).to_string()),
            created_at_db: None,
            updated_at: None,
            append_system_prompt: None,
            sandbox_started_at: None,
            sandbox_desired_state: crate::task::models::SandboxDesiredState::Running,
            sandbox_stop_reason: None,
            sandbox_desired_state_updated_at: epoch(),
            headless: false,
            ttl_minutes: 0,
            inactive_ttl_days: 0,
            trigger_id: None,
            workspace_id: None,
            disabled_servers: None,
        }
    }

    fn ctx(user_id: &str, team_ids: &[&str], is_admin: bool) -> ScopeContext {
        ScopeContext {
            user_id: uid(user_id).to_string(),
            team_ids: team_ids.iter().map(|t| (*t).to_string()).collect(),
            is_admin,
        }
    }

    /// A task repo whose scoped `find_by_id` always yields `task` (models a task
    /// the caller can *read*); control is decided downstream by the team role.
    fn tasks_returning(task: Option<Task>) -> MockTaskRepository {
        let mut m = MockTaskRepository::new();
        m.expect_find_by_id()
            .returning(move |_, _| Ok(task.clone()));
        m
    }

    #[tokio::test]
    async fn read_unreadable_task_is_not_found() {
        // Scoped read returns None → NotFound, never leaking the task's existence.
        let tasks = tasks_returning(None);
        let scope = Some(ctx("intruder", &[], false));
        let err = authorize_task_read_inner(Some(&tasks), &scope, "task-1")
            .await
            .unwrap_err();
        assert!(matches!(err, SandboxError::NotFound(_)));
    }

    #[tokio::test]
    async fn read_allows_when_scoped_lookup_returns_task() {
        let tasks = tasks_returning(Some(task_fixture(Scope::User, Some("owner"), None)));
        let scope = Some(ctx("owner", &[], false));
        assert!(authorize_task_read_inner(Some(&tasks), &scope, "task-1")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn read_without_identity_is_forbidden() {
        // No identity means the request never authenticated — fail closed
        // rather than resolving the task under an unscoped system context.
        let tasks = tasks_returning(Some(task_fixture(Scope::User, Some("someone"), None)));
        assert!(matches!(
            authorize_task_read_inner(Some(&tasks), &None, "task-1").await,
            Err(SandboxError::Forbidden(_))
        ));
    }

    #[tokio::test]
    async fn read_without_repo_is_internal_error() {
        let scope = Some(ctx("u", &[], false));
        let err = authorize_task_read_inner(None, &scope, "task-1")
            .await
            .unwrap_err();
        assert!(matches!(err, SandboxError::Internal(_)));
    }

    #[tokio::test]
    async fn control_owner_of_user_task_allowed() {
        let tasks = tasks_returning(Some(task_fixture(Scope::User, Some("owner"), None)));
        let teams = StubTeamRepo::with_roles(&[]);
        let scope = Some(ctx("owner", &[], false));
        assert!(
            authorize_task_control_inner(Some(&tasks), Some(&teams), &scope, "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn control_readable_but_non_managing_member_is_forbidden() {
        // Team task the caller can READ (plain member) but not CONTROL.
        let tasks = tasks_returning(Some(task_fixture(Scope::Team, None, Some("team-1"))));
        let teams = StubTeamRepo::with_roles(&[("member-user", "member")]);
        let scope = Some(ctx("member-user", &["team-1"], false));
        let err = authorize_task_control_inner(Some(&tasks), Some(&teams), &scope, "task-1")
            .await
            .unwrap_err();
        assert!(matches!(err, SandboxError::Forbidden(_)));
    }

    #[tokio::test]
    async fn control_team_task_creator_with_member_role_allowed() {
        // Production shape for member-created team tasks: `owner_user_id` is
        // None and `created_by` records the creator, who keeps control of
        // their own task (and its sandbox) with a plain member role.
        let mut task = task_fixture(Scope::Team, None, Some("team-1"));
        task.created_by = Some(uid("creator").to_string());
        let tasks = tasks_returning(Some(task));
        let teams = StubTeamRepo::with_roles(&[("creator", "member")]);
        let scope = Some(ctx("creator", &["team-1"], false));
        assert!(
            authorize_task_control_inner(Some(&tasks), Some(&teams), &scope, "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn control_managing_member_of_team_task_allowed() {
        let tasks = tasks_returning(Some(task_fixture(Scope::Team, None, Some("team-1"))));
        let teams = StubTeamRepo::with_roles(&[("lead", "admin")]);
        let scope = Some(ctx("lead", &["team-1"], false));
        assert!(
            authorize_task_control_inner(Some(&tasks), Some(&teams), &scope, "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn control_unreadable_task_is_not_found_not_forbidden() {
        // Can't even read the task → NotFound (no existence leak) before the
        // control check runs.
        let tasks = tasks_returning(None);
        let teams = StubTeamRepo::with_roles(&[]);
        let scope = Some(ctx("intruder", &[], false));
        let err = authorize_task_control_inner(Some(&tasks), Some(&teams), &scope, "task-1")
            .await
            .unwrap_err();
        assert!(matches!(err, SandboxError::NotFound(_)));
    }

    #[tokio::test]
    async fn control_admin_bypasses_team_role() {
        let tasks = tasks_returning(Some(task_fixture(Scope::Team, None, Some("team-1"))));
        let teams = StubTeamRepo::with_roles(&[]);
        let scope = Some(ctx("root", &[], true));
        assert!(
            authorize_task_control_inner(Some(&tasks), Some(&teams), &scope, "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn control_without_identity_is_forbidden() {
        let tasks = tasks_returning(Some(task_fixture(Scope::User, Some("someone"), None)));
        let teams = StubTeamRepo::with_roles(&[]);
        assert!(matches!(
            authorize_task_control_inner(Some(&tasks), Some(&teams), &None, "task-1").await,
            Err(SandboxError::Forbidden(_))
        ));
    }
}
