//! Sandbox handler functions.
//!
//! Extracted from routes.rs for consistency with other modules
//! (task/, message/, agent/, etc.).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{Stream, StreamExt};
use serde::Deserialize;
use tracing::{error, info, warn};

use super::access::LogOptions;
use super::error::SandboxError;
use super::lifecycle::{ArchiveRetention, Lifecycle};
use super::models::*;
use super::routes::SandboxApiState;
use super::schemas::*;
use moatless_types::sandbox::{SandboxDesiredState, SandboxStopReason};
use moatless_types::access_control::ScopeContext;
use moatless_identity::extractors::OptionalScope;
use moatless_tasks::task::dao::TaskDao;
use moatless_tasks::task::models::Task;

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
        (status = 400, description = "Task is Closed — start not allowed", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "Another lifecycle operation is in progress", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Provisioning failed", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[tracing::instrument(skip_all, fields(task_id = %task_id))]
pub async fn provision_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> Result<Json<ProvisionResponse>, SandboxError> {
    info!(task_id = %task_id, "POST /provision");
    authorize_task_control(&state, &scope, &task_id).await?;

    // Lifecycle owns plan construction, the Running intent, and the start
    // timestamp; the runtime receives a fresh request from DB state.
    let outcome = require_lifecycle(&state)?.start(&task_id).await?;
    info!(task_id = %task_id, "Provision (start) succeeded");
    Ok(Json(ProvisionResponse {
        status: outcome.observed.sandbox_status(),
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
        (status = 200, description = "Task live status", body = crate::live_status::LiveStatus),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[tracing::instrument(skip_all, fields(task_id = %task_id))]
pub async fn live_status_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let task = match authorize_task_read(&state, &scope, &task_id).await {
        Ok(task) => task,
        Err(e) => return e.into_response(),
    };
    let desired_state = match resolve_desired_state(&state, &task_id).await {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };

    match state
        .observer
        .live_status(&task_id, task.status, desired_state)
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
        (status = 503, description = "Sandbox not ready", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[tracing::instrument(skip_all, fields(task_id = %task_id))]
pub async fn sandbox_status_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let task = match authorize_task_read(&state, &scope, &task_id).await {
        Ok(task) => task,
        Err(e) => return e.into_response(),
    };
    let desired_state = match resolve_desired_state(&state, &task_id).await {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };

    match state
        .observer
        .sandbox_status(&task_id, task.status, desired_state)
        .await
    {
        Ok(mut resp) => {
            // Surface the recorded lifecycle intent and image freshness so the
            // UI can tell whether a redeploy would move the sandbox to a newer
            // image. The "latest" image is what a fresh (re)deploy would use:
            // the per-repo `docker_image` override, else the configured default.
            resp.sandbox_desired_state =
                Some(desired_state);

            if let Some(lifecycle) = state.lifecycle.as_deref() {
                match lifecycle.expected_sandbox_image(&task_id).await {
                    Ok(latest) => {
                        resp.image_up_to_date = resp
                            .current_sandbox_image
                            .as_ref()
                            .map(|current| current == &latest);
                        resp.latest_sandbox_image = Some(latest);
                    }
                    Err(e) => {
                        warn!(
                            task_id,
                            error = %e,
                            "Failed to resolve expected sandbox image for status"
                        );
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
        (status = 409, description = "Another lifecycle operation is in progress", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Stop failed", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[tracing::instrument(skip_all, fields(task_id = %task_id))]
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
        (status = 400, description = "Task is Closed — restart not allowed", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "Another lifecycle operation is in progress", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Restart failed", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[tracing::instrument(skip_all, fields(task_id = %task_id))]
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
        (status = 400, description = "Task is Closed — redeploy not allowed", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "Another lifecycle operation is in progress", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Redeploy failed", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[tracing::instrument(skip_all, fields(task_id = %task_id))]
pub async fn redeploy_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> Result<Json<ProvisionResponse>, SandboxError> {
    info!(task_id = %task_id, "POST /redeploy");
    authorize_task_control(&state, &scope, &task_id).await?;

    // Lifecycle owns plan construction and detaches internally; the runtime
    // receives a fresh request from DB state.
    let outcome = require_lifecycle(&state)?.redeploy(&task_id).await?;
    info!(task_id = %task_id, "Redeploy succeeded");
    Ok(Json(ProvisionResponse {
        status: outcome.observed.sandbox_status(),
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
        (status = 409, description = "Another lifecycle operation is in progress", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Cleanup failed", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[tracing::instrument(skip_all, fields(task_id = %task_id))]
pub async fn remove_handler(
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
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn server_status_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let task = match authorize_task_read(&state, &scope, &task_id).await {
        Ok(task) => task,
        Err(e) => {
            return e.into_response();
        }
    };
    let desired_state = match resolve_desired_state(&state, &task_id).await {
        Ok(desired_state) => desired_state,
        Err(e) => return e.into_response(),
    };
    let status = match state
        .observer
        .sandbox_status(&task_id, task.status, desired_state)
        .await
    {
        Ok(status) => status,
        Err(e) => {
            error!(task_id = %task_id, error = %e, "server status observation failed");
            return e.into_response();
        }
    };
    // No `config_override` here: overrides live only on the sandbox's workspace
    // volume, and this path resolves status from the K8s API with no sandbox
    // round-trip. Adding one would put a gRPC call on a polled endpoint to serve
    // a debugging field. `GET /servers/config` reports overrides instead.
    let servers = status
        .servers
        .into_iter()
        .map(|server| ServerSnapshot {
            name: server.name,
            label: server.label,
            port: server.port,
            builtin: false,
            default: server.default,
            enable_inspector: server.capabilities.inspector,
            status: server.status,
            error: server.error,
            pid: None,
        })
        .collect();
    Json(ServerStatusResponse { servers }).into_response()
}

/// Query parameters for the server logs endpoint.
#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    /// Fetch logs from the previous container instance (useful for crashed containers).
    #[serde(default)]
    pub previous: bool,
    /// Hold the stream open and append new lines. Defaults to true.
    #[serde(default = "default_follow")]
    pub follow: bool,
    /// How many lines back from the end to start.
    #[serde(default = "default_tail")]
    pub tail: i64,
}

fn default_follow() -> bool {
    LogOptions::default().follow
}

fn default_tail() -> i64 {
    LogOptions::default().tail_lines
}

/// Stream server logs via SSE.
///
/// Returns a Server-Sent Events stream of log lines for the given server.
/// Pass `?previous=true` to fetch logs from a crashed/restarted container, and
/// `?follow=false` to end the stream after the buffered lines instead of
/// waiting for more. The response body cannot be represented in OpenAPI (SSE
/// streaming).
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/servers/{server_name}/logs",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("server_name" = String, Path, description = "Server name"),
        ("previous" = bool, Query, description = "Fetch logs from previous container instance"),
        ("follow" = Option<bool>, Query, description = "Keep the stream open for new lines (default true)"),
        ("tail" = Option<i64>, Query, description = "Number of trailing lines to start from (default 500)")
    ),
    responses(
        (status = 200, description = "SSE stream of server log lines"),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn server_logs_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path((task_id, server_name)): Path<(String, String)>,
    Query(query): Query<LogsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, SandboxError>>>, SandboxError> {
    authorize_task_read(&state, &scope, &task_id).await?;
    let lines = state
        .observer
        .server_logs(
            &task_id,
            &server_name,
            LogOptions {
                previous: query.previous,
                follow: query.follow,
                tail_lines: query.tail,
            },
        )
        .await?;
    // The observer produces lines; SSE framing is this boundary's business and
    // not the observer's, because the same lines also leave over the T3 RPC
    // socket as chunk values.
    let events = lines.map(|line| line.map(|line| Event::default().data(line)));
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

// =============================================================================
// Live server config
// =============================================================================
//
// These three endpoints reconfigure and restart a single preview server without
// rebuilding the pod, so — unlike `restart`/`redeploy` — they are safe to call
// from inside the sandbox: the agent container is never touched.
//
// **The backend persists nothing here.** An override is workspace state: the
// sandbox writes it to control files under `WORKSPACE_DIR/.moatless/servers/`,
// which ride the same volume as the repo checkouts and therefore survive a
// stop/start, a redeploy, and the reaper's idle-stop wherever the checkouts do.
// The `ephemeral` tier keeps neither, which is that tier's contract.
//
// The consequence for these handlers is that the sandbox is the only source of
// truth, so a partial update is a read-modify-write over gRPC. That is not
// atomic against a concurrent PATCH to the same server. This is a per-task
// debugging surface, not a concurrent one.

/// List the overrides currently in effect, read off the sandbox's workspace.
///
/// Separate from `GET /servers` because it needs a live sandbox: there is no
/// persisted copy to fall back on, so a stopped sandbox has no answer.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/servers/config",
    tag = "sandbox",
    params(("task_id" = String, Path, description = "Task identifier")),
    responses(
        (status = 200, description = "Overrides in effect", body = ServerOverridesResponse),
        (status = 503, description = "Sandbox not reachable", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_server_config_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> Result<Json<ServerOverridesResponse>, SandboxError> {
    authorize_task_read(&state, &scope, &task_id).await?;
    let client = state.connector.sandbox_client(&task_id).await?;
    let overrides = client
        .get_server_config(&task_id)
        .await
        .map_err(|e| crate::client::classify_grpc_error(e, "GetServerConfig"))?;

    Ok(Json(ServerOverridesResponse {
        overrides: overrides
            .into_iter()
            .map(
                |(server_name, (config_override, generation))| ServerOverrideEntry {
                    server_name,
                    config_override: server_override_from_client(config_override),
                    generation,
                },
            )
            .collect(),
    }))
}

/// Update one preview server's start command / env and (by default) restart it.
#[utoipa::path(
    patch,
    path = "/api/sandbox/v1/tasks/{task_id}/servers/{server_name}",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("server_name" = String, Path, description = "Server name")
    ),
    request_body = UpdateServerConfigRequest,
    responses(
        (status = 200, description = "Override applied", body = ServerConfigResponse),
        (status = 400, description = "Invalid env key or server name", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "No such server on this task", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "Field requires a redeploy to change", body = moatless_api_schemas::error::ErrorBody),
        (status = 503, description = "Sandbox not reachable", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn update_server_config_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path((task_id, server_name)): Path<(String, String)>,
    Json(req): Json<UpdateServerConfigRequest>,
) -> Result<Json<ServerConfigResponse>, SandboxError> {
    info!(task_id = %task_id, server = %server_name, "PATCH /servers/{server_name}");
    let task = authorize_task_control(&state, &scope, &task_id).await?;

    if req.port.is_some() {
        return Err(SandboxError::Conflict(
            "changing a server's port requires a redeploy — it is wired into the Service and \
             IngressRoute at provision time (POST /redeploy)"
                .into(),
        ));
    }
    if req.docker_image.is_some() {
        return Err(SandboxError::Conflict(
            "changing a server's image requires a redeploy (POST /redeploy)".into(),
        ));
    }

    require_known_server(&state, &task, &server_name).await?;

    let client = state.connector.sandbox_client(&task_id).await?;
    let mut entry = read_override(&client, &task_id, &server_name).await?;

    if let Some(cmds) = req.start_commands {
        entry.start_command = if cmds.is_empty() {
            None
        } else {
            Some(cmds.join(" && "))
        };
    }
    if let Some(env) = req.env {
        for (key, value) in env {
            validate_env_key(&key)?;
            // A key can't be both exported and unset — the later `set` wins.
            entry.unset_env.retain(|k| k != &key);
            entry.env.insert(key, value);
        }
    }
    if let Some(unset) = req.unset_env {
        for key in unset {
            validate_env_key(&key)?;
            entry.env.remove(&key);
            if !entry.unset_env.contains(&key) {
                entry.unset_env.push(key);
            }
        }
    }

    // An emptied entry writes as "no override" — `apply` removes the control
    // files rather than leaving empty ones, so it has one representation.
    apply_override(&client, &task_id, &server_name, entry, req.restart).await
}

/// Restart one preview server in place, leaving its override as-is.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/servers/{server_name}/restart",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("server_name" = String, Path, description = "Server name")
    ),
    responses(
        (status = 200, description = "Server restarted", body = ServerConfigResponse),
        (status = 404, description = "No such server on this task", body = moatless_api_schemas::error::ErrorBody),
        (status = 503, description = "Sandbox not reachable", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn restart_server_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path((task_id, server_name)): Path<(String, String)>,
) -> Result<Json<ServerConfigResponse>, SandboxError> {
    info!(task_id = %task_id, server = %server_name, "POST /servers/{server_name}/restart");
    let task = authorize_task_control(&state, &scope, &task_id).await?;
    require_known_server(&state, &task, &server_name).await?;

    // Re-send the override the sandbox already holds: `apply` is idempotent, and
    // the generation bump is the only part the sidecar reacts to.
    let client = state.connector.sandbox_client(&task_id).await?;
    let current = read_override(&client, &task_id, &server_name).await?;
    apply_override(&client, &task_id, &server_name, current, true).await
}

/// Drop a server's override and restart it on the workspace's configuration.
#[utoipa::path(
    delete,
    path = "/api/sandbox/v1/tasks/{task_id}/servers/{server_name}/override",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("server_name" = String, Path, description = "Server name")
    ),
    responses(
        (status = 200, description = "Override cleared", body = ServerConfigResponse),
        (status = 404, description = "No such server on this task", body = moatless_api_schemas::error::ErrorBody),
        (status = 503, description = "Sandbox not reachable", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn reset_server_config_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path((task_id, server_name)): Path<(String, String)>,
) -> Result<Json<ServerConfigResponse>, SandboxError> {
    info!(task_id = %task_id, server = %server_name, "DELETE /servers/{server_name}/override");
    let task = authorize_task_control(&state, &scope, &task_id).await?;
    require_known_server(&state, &task, &server_name).await?;

    // An empty override is exactly "reset": `apply` removes the control files.
    let client = state.connector.sandbox_client(&task_id).await?;
    apply_override(
        &client,
        &task_id,
        &server_name,
        ServerOverride::default(),
        true,
    )
    .await
}

/// Read one server's current override off the sandbox's workspace volume.
///
/// Absent means no override, which is the same value as an empty one — callers
/// merge into it either way.
async fn read_override(
    client: &crate::client::SandboxClient,
    task_id: &str,
    server_name: &str,
) -> Result<ServerOverride, SandboxError> {
    let mut overrides = client
        .get_server_config(task_id)
        .await
        .map_err(|e| crate::client::classify_grpc_error(e, "GetServerConfig"))?;
    Ok(overrides
        .remove(server_name)
        .map(|(config, _generation)| server_override_from_client(config))
        .unwrap_or_default())
}

/// Push one server's complete override to the sandbox, optionally restarting it.
async fn apply_override(
    client: &crate::client::SandboxClient,
    task_id: &str,
    server_name: &str,
    effective: ServerOverride,
    restart: bool,
) -> Result<Json<ServerConfigResponse>, SandboxError> {
    let client_override = client_server_override(&effective);
    let generation = client
        .apply_server_config(task_id, server_name, &client_override, restart)
        .await
        .map_err(|e| crate::client::classify_grpc_error(e, "ApplyServerConfig"))?;

    info!(
        task_id = %task_id,
        server = %server_name,
        generation,
        restart,
        "Applied live server config"
    );
    Ok(Json(ServerConfigResponse {
        server_name: server_name.to_string(),
        config_override: effective,
        generation,
        restarted: restart,
    }))
}

fn server_override_from_client(value: crate::client::ServerOverride) -> ServerOverride {
    ServerOverride {
        start_command: value.start_command,
        env: value.env,
        unset_env: value.unset_env,
    }
}

fn client_server_override(value: &ServerOverride) -> crate::client::ServerOverride {
    crate::client::ServerOverride {
        start_command: value.start_command.clone(),
        env: value.env.clone(),
        unset_env: value.unset_env.clone(),
    }
}

/// 404 unless the task actually has a server by this name.
///
/// Resolved from live status, which is config-first, so a server that has not
/// started yet still counts.
async fn require_known_server(
    state: &SandboxApiState,
    task: &Task,
    server_name: &str,
) -> Result<(), SandboxError> {
    let task_id = &task.task_id;
    let desired_state = resolve_desired_state(state, task_id).await?;
    let status = state
        .observer
        .sandbox_status(task_id, task.status, desired_state)
        .await?;
    if status.servers.iter().any(|s| s.name == server_name) {
        return Ok(());
    }
    Err(SandboxError::NotFound(format!(
        "no server '{server_name}' on task {task_id}; available: {}",
        status
            .servers
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Env keys must be shell-legal and must not collide with names the platform
/// controls — the runtime trio the container spec injects, plus the reserved
/// secret keys.
fn validate_env_key(key: &str) -> Result<(), SandboxError> {
    if !moatless_identity::secret::models::is_valid_secret_key(key) {
        return Err(SandboxError::BadRequest(format!(
            "invalid env key '{key}': expected ^[A-Z_][A-Z0-9_]*$"
        )));
    }
    if moatless_identity::secret::models::is_reserved_key(key)
        || matches!(
            key,
            "PORT" | "WORKSPACE_DIR" | "WORKSPACE_PATH" | "DATA_DIR"
        )
    {
        return Err(SandboxError::BadRequest(format!(
            "env key '{key}' is reserved by the platform"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_env_key;
    use crate::error::SandboxError;

    #[test]
    fn server_env_overrides_reject_runtime_paths() {
        for key in ["PORT", "WORKSPACE_DIR", "WORKSPACE_PATH", "DATA_DIR"] {
            assert!(
                matches!(validate_env_key(key), Err(SandboxError::BadRequest(_))),
                "{key} should be reserved"
            );
        }
        assert!(validate_env_key("EXTRA_FLAG").is_ok());
    }
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
/// owner/creator of the task or a platform admin. Read-only viewers get
/// `Forbidden`.
pub(super) async fn authorize_task_control(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<Task, SandboxError> {
    authorize_task_control_inner(state.tasks.as_deref(), scope, task_id).await
}

/// Core of [`authorize_task_read`] against explicit dependencies (so it can be
/// unit-tested without runtime infrastructure).
async fn authorize_task_read_inner(
    tasks: Option<&dyn TaskDao>,
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
        .map_err(|source| SandboxError::Database {
            context: "looking up the task for authorization",
            source,
        })?
        .ok_or_else(|| SandboxError::NotFound(format!("task {task_id}")))
}

/// Core of [`authorize_task_control`] against explicit dependencies.
async fn authorize_task_control_inner(
    tasks: Option<&dyn TaskDao>,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<Task, SandboxError> {
    let task = authorize_task_read_inner(tasks, scope, task_id).await?;

    let Some(ctx) = scope.as_ref() else {
        return Err(SandboxError::Forbidden("authentication required".into()));
    };

    if !ctx.can_control(&task.scoped_resource()) {
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

/// The desired Sandbox state for a Task, read from the Sandbox row through the
/// lifecycle store. Absence of a row reads as `Stopped` (never requested).
async fn resolve_desired_state(
    state: &SandboxApiState,
    task_id: &str,
) -> Result<SandboxDesiredState, SandboxError> {
    Ok(require_lifecycle(state)?.desired_state(task_id).await?)
}

#[cfg(test)]
mod authz_tests {
    //! Unit tests for the per-task sandbox authorization gate. The scoped-read
    //! SQL filter itself is covered by `scope.rs`; these exercise how the
    //! handlers compose read + control on the task a scoped read returned.
    use super::*;

    use moatless_types::access_control::Scope;

    use moatless_tasks::task::dao::MockTaskDao;

    /// Deterministic uuid for a readable test name: distinct names map to
    /// distinct ids, equal names to equal ids.
    fn uid(name: &str) -> uuid::Uuid {
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
    }

    /// A `Task` fixture with the given scope/owner; all other columns are
    /// inert defaults irrelevant to the access check.
    fn task_fixture(scope: Scope, owner_user_id: Option<&str>) -> Task {
        Task {
            task_id: "task-1".to_string(),
            short_id: None,
            created_at: chrono::Utc::now(),
            last_message_at: chrono::Utc::now(),
            agent_type: moatless_types::agent::AgentType("claude-code".to_string()),
            repository_id: "repo-1".to_string(),
            status: moatless_types::task::TaskStatus::Open,
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
            created_by: owner_user_id.map(|u| uid(u).to_string()),
            visibility: moatless_types::task::Visibility::Private,
            scope,
            owner_user_id: owner_user_id.map(|u| uid(u).to_string()),
            updated_at: None,
            append_system_prompt: None,
            headless: false,
            ttl_minutes: 0,
            inactive_ttl_days: 0,
            loop_id: None,
            workspace_id: None,
            sandbox_id: None,
            checkout_mode: None,
            skills: Vec::new(),
        }
    }

    fn ctx(user_id: &str, is_admin: bool) -> ScopeContext {
        ScopeContext {
            user_id: uid(user_id).to_string(),
            is_admin,
        }
    }

    /// A task repo whose scoped `find_by_id` always yields `task` (models a task
    /// the caller can *read*); control is decided downstream by `can_control`.
    fn tasks_returning(task: Option<Task>) -> MockTaskDao {
        let mut m = MockTaskDao::new();
        m.expect_find_by_id()
            .returning(move |_, _| Ok(task.clone()));
        m
    }

    #[tokio::test]
    async fn read_unreadable_task_is_not_found() {
        // Scoped read returns None → NotFound, never leaking the task's existence.
        let tasks = tasks_returning(None);
        let scope = Some(ctx("intruder", false));
        let err = authorize_task_read_inner(Some(&tasks), &scope, "task-1")
            .await
            .unwrap_err();
        assert!(matches!(err, SandboxError::NotFound(_)));
    }

    #[tokio::test]
    async fn read_allows_when_scoped_lookup_returns_task() {
        let tasks = tasks_returning(Some(task_fixture(Scope::User, Some("owner"))));
        let scope = Some(ctx("owner", false));
        assert!(
            authorize_task_read_inner(Some(&tasks), &scope, "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn read_without_identity_is_forbidden() {
        // No identity means the request never authenticated — fail closed
        // rather than resolving the task under an unscoped system context.
        let tasks = tasks_returning(Some(task_fixture(Scope::User, Some("someone"))));
        assert!(matches!(
            authorize_task_read_inner(Some(&tasks), &None, "task-1").await,
            Err(SandboxError::Forbidden(_))
        ));
    }

    #[tokio::test]
    async fn read_without_repo_is_internal_error() {
        let scope = Some(ctx("u", false));
        let err = authorize_task_read_inner(None, &scope, "task-1")
            .await
            .unwrap_err();
        assert!(matches!(err, SandboxError::Internal(_)));
    }

    #[tokio::test]
    async fn control_owner_of_user_task_allowed() {
        let tasks = tasks_returning(Some(task_fixture(Scope::User, Some("owner"))));
        let scope = Some(ctx("owner", false));
        assert!(
            authorize_task_control_inner(Some(&tasks), &scope, "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn control_non_owner_of_user_task_is_forbidden() {
        // A user-scoped task belongs only to its owner; a stranger is denied.
        let tasks = tasks_returning(Some(task_fixture(Scope::User, Some("owner"))));
        let scope = Some(ctx("stranger", false));
        let err = authorize_task_control_inner(Some(&tasks), &scope, "task-1")
            .await
            .unwrap_err();
        assert!(matches!(err, SandboxError::Forbidden(_)));
    }

    #[tokio::test]
    async fn control_unreadable_task_is_not_found_not_forbidden() {
        // Can't even read the task → NotFound (no existence leak) before the
        // control check runs.
        let tasks = tasks_returning(None);
        let scope = Some(ctx("intruder", false));
        let err = authorize_task_control_inner(Some(&tasks), &scope, "task-1")
            .await
            .unwrap_err();
        assert!(matches!(err, SandboxError::NotFound(_)));
    }

    #[tokio::test]
    async fn control_admin_bypasses_scope() {
        // A platform admin controls any task regardless of scope/owner.
        let tasks = tasks_returning(Some(task_fixture(Scope::User, Some("other-user"))));
        let scope = Some(ctx("root", true));
        assert!(
            authorize_task_control_inner(Some(&tasks), &scope, "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn control_without_identity_is_forbidden() {
        let tasks = tasks_returning(Some(task_fixture(Scope::User, Some("someone"))));
        assert!(matches!(
            authorize_task_control_inner(Some(&tasks), &None, "task-1").await,
            Err(SandboxError::Forbidden(_))
        ));
    }
}
