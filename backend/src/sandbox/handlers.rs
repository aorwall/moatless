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
use super::models::*;
use super::routes::SandboxApiState;
use super::schemas::*;
use crate::task::dao::TaskPatch;
use crate::task::models::{SandboxDesiredState, SandboxStopReason};

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
    Path(task_id): Path<String>,
) -> Result<Json<ProvisionResponse>, SandboxError> {
    info!(task_id = %task_id, "POST /provision");

    let (tasks, repos, provider_tokens, plugin_repos) = match (
        &state.tasks,
        &state.repos,
        &state.provider_tokens,
        &state.plugin_repos,
    ) {
        (Some(t), Some(r), Some(pt), Some(pr)) => {
            (t.as_ref(), r.as_ref(), pt.as_ref(), pr.as_ref())
        }
        _ => {
            return Err(SandboxError::Internal(
                "Database not available for provisioning".into(),
            ));
        }
    };

    let request = super::auto_provision::build_provision_request(
        tasks,
        repos,
        provider_tokens,
        plugin_repos,
        &task_id,
    )
    .await?;

    match state.provider.provision(&task_id, &request).await {
        Ok(response) => {
            info!(task_id = %task_id, "Provision succeeded");
            record_sandbox_started(&state, &task_id).await;
            Ok(Json(response))
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Provision failed");
            Err(e)
        }
    }
}

/// Get full sandbox status including servers and agent state.
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
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    // Resolve server configs from DB
    let server_configs = resolve_server_configs(&state, &task_id).await;

    match state
        .provider
        .sandbox_status(&task_id, &server_configs)
        .await
    {
        Ok(mut resp) => {
            // If sandbox doesn't exist and the task is closed, report as Removed
            if resp.environment_status == SandboxStatus::NotCreated {
                if let Some(ref tasks) = state.tasks {
                    if let Ok(Some(task)) = tasks.find_by_id_internal(&task_id).await {
                        if task.status == crate::task::models::TaskStatus::Closed {
                            resp.environment_status = SandboxStatus::Removed;
                        }
                    }
                }
            }
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
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /stop");
    match state.provider.stop(&task_id).await {
        Ok(()) => {
            record_sandbox_stopped(&state, &task_id, SandboxStopReason::User).await;
            info!(task_id = %task_id, "Stop succeeded");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Stop failed");
            Err(e)
        }
    }
}

/// Run a multi-step lifecycle sequence on a detached task so it completes even
/// if the HTTP client disconnects. Axum drops the handler future on disconnect;
/// a mid-flight drop of a scale-to-0 -> wait -> scale-to-1 sequence leaves the
/// sandbox half-transitioned (e.g. permanently stuck at 0 replicas).
async fn run_detached<T: Send + 'static>(
    fut: impl std::future::Future<Output = Result<T, SandboxError>> + Send + 'static,
) -> Result<T, SandboxError> {
    tokio::spawn(fut)
        .await
        .map_err(|e| SandboxError::Internal(format!("lifecycle task panicked: {e}")))?
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
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /restart");
    let outcome = {
        let provider = state.provider.clone();
        let task_id = task_id.clone();
        run_detached(async move { provider.restart(&task_id).await }).await
    };
    match outcome {
        Ok(()) => {
            info!(task_id = %task_id, "Restart succeeded");
            record_sandbox_started(&state, &task_id).await;
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Restart failed");
            Err(e)
        }
    }
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
    Path(task_id): Path<String>,
) -> Result<Json<ProvisionResponse>, SandboxError> {
    info!(task_id = %task_id, "POST /redeploy");

    let (tasks, repos, provider_tokens, plugin_repos) = match (
        &state.tasks,
        &state.repos,
        &state.provider_tokens,
        &state.plugin_repos,
    ) {
        (Some(t), Some(r), Some(pt), Some(pr)) => {
            (t.as_ref(), r.as_ref(), pt.as_ref(), pr.as_ref())
        }
        _ => {
            return Err(SandboxError::Internal(
                "Database not available for redeployment".into(),
            ));
        }
    };

    let request = super::auto_provision::build_provision_request(
        tasks,
        repos,
        provider_tokens,
        plugin_repos,
        &task_id,
    )
    .await?;

    let outcome = {
        let provider = state.provider.clone();
        let task_id = task_id.clone();
        run_detached(async move { provider.redeploy(&task_id, &request).await }).await
    };
    match outcome {
        Ok(response) => {
            info!(task_id = %task_id, "Redeploy succeeded");
            record_sandbox_started(&state, &task_id).await;
            Ok(Json(response))
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Redeploy failed");
            Err(e)
        }
    }
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
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /cleanup");
    match state.provider.cleanup(&task_id, false).await {
        Ok(()) => {
            record_sandbox_removed(&state, &task_id).await;
            info!(task_id = %task_id, "Cleanup succeeded");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Cleanup failed");
            Err(e)
        }
    }
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
    Path(task_id): Path<String>,
) -> impl IntoResponse {
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
    Path((task_id, server_name)): Path<(String, String)>,
    Query(query): Query<LogsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, SandboxError>>>, SandboxError> {
    let stream = state
        .provider
        .server_logs(&task_id, &server_name, query.previous)
        .await?;
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// =============================================================================
// Private helpers
// =============================================================================

/// Update `sandbox_started_at` to prevent the idle reaper from killing
/// a just-started sandbox based on stale message timestamps.
async fn record_sandbox_started(state: &SandboxApiState, task_id: &str) {
    if let Some(ref tasks) = state.tasks {
        let patch = TaskPatch {
            sandbox_started_at: Some(Utc::now()),
            sandbox_desired_state: Some(SandboxDesiredState::Running),
            sandbox_stop_reason: Some(None),
            ..Default::default()
        };
        if let Err(e) = tasks.update_task(task_id, &patch).await {
            warn!(task_id = %task_id, error = %e, "Failed to update sandbox_started_at");
        }
    }
}

async fn record_sandbox_stopped(state: &SandboxApiState, task_id: &str, reason: SandboxStopReason) {
    if let Some(ref tasks) = state.tasks {
        let patch = TaskPatch {
            sandbox_desired_state: Some(SandboxDesiredState::Stopped),
            sandbox_stop_reason: Some(Some(reason)),
            ..Default::default()
        };
        if let Err(e) = tasks.update_task(task_id, &patch).await {
            warn!(task_id = %task_id, error = %e, "Failed to update sandbox stopped intent");
        }
    }
}

async fn record_sandbox_removed(state: &SandboxApiState, task_id: &str) {
    if let Some(ref tasks) = state.tasks {
        let patch = TaskPatch {
            sandbox_desired_state: Some(SandboxDesiredState::Removed),
            sandbox_stop_reason: Some(Some(SandboxStopReason::Cleanup)),
            ..Default::default()
        };
        if let Err(e) = tasks.update_task(task_id, &patch).await {
            warn!(task_id = %task_id, error = %e, "Failed to update sandbox removed intent");
        }
    }
}

/// Resolve server configs from DB via the SandboxApiState repositories.
async fn resolve_server_configs(state: &SandboxApiState, task_id: &str) -> Vec<ServerConfig> {
    if let (Some(ref tasks), Some(ref repos)) = (&state.tasks, &state.repos) {
        super::auto_provision::get_server_configs_for_task(tasks.as_ref(), repos.as_ref(), task_id)
            .await
    } else {
        warn!(task_id = %task_id, "No task/repo repositories available for server config resolution");
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
