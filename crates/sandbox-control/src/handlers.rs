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
use super::lifecycle::{ArchiveRetention, Lifecycle, TaskIntent};
use super::models::*;
use super::routes::SandboxApiState;
use super::schemas::*;
use moatless_types::sandbox::SandboxStopReason;
use moatless_types::access_control::ScopeContext;
use moatless_types::task::Visibility;
use moatless_identity::extractors::OptionalScope;

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
    let intent = match authorize_task_read(&state, &scope, &task_id).await {
        Ok(intent) => intent,
        Err(e) => return e.into_response(),
    };

    match state
        .observer
        .live_status(&task_id, intent.status, intent.desired_state)
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
    let intent = match authorize_task_read(&state, &scope, &task_id).await {
        Ok(intent) => intent,
        Err(e) => return e.into_response(),
    };
    let desired_state = intent.desired_state;

    match state
        .observer
        .sandbox_status(&task_id, intent.status, desired_state)
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
    let intent = match authorize_task_read(&state, &scope, &task_id).await {
        Ok(intent) => intent,
        Err(e) => {
            return e.into_response();
        }
    };
    let status = match state
        .observer
        .sandbox_status(&task_id, intent.status, intent.desired_state)
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
    let intent = authorize_task_control(&state, &scope, &task_id).await?;

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

    require_known_server(&state, &intent, &task_id, &server_name).await?;

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
    let intent = authorize_task_control(&state, &scope, &task_id).await?;
    require_known_server(&state, &intent, &task_id, &server_name).await?;

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
    let intent = authorize_task_control(&state, &scope, &task_id).await?;
    require_known_server(&state, &intent, &task_id, &server_name).await?;

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
    intent: &TaskIntent,
    task_id: &str,
    server_name: &str,
) -> Result<(), SandboxError> {
    let status = state
        .observer
        .sandbox_status(task_id, intent.status, intent.desired_state)
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

/// Load the Task's Sandbox membership and require *read* access to it.
///
/// Returns `NotFound` when the caller may not read it, so the existence of
/// another tenant's Sandbox never leaks. The returned [`TaskIntent`] is the one
/// read every handler works from: it carries the Task's status and the
/// Sandbox's desired state alongside the access-control pair.
pub(super) async fn authorize_task_read(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<TaskIntent, SandboxError> {
    let intent = require_lifecycle(state)?.intent(task_id).await?;
    ensure_can_read(scope, &intent, task_id)?;
    Ok(intent)
}

/// Like [`authorize_task_read`], but additionally requires *control* access
/// (the same policy the task HTTP API and sandbox gRPC server enforce): the
/// owner of the Sandbox or a platform admin. Read-only viewers get `Forbidden`.
pub(super) async fn authorize_task_control(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<TaskIntent, SandboxError> {
    let intent = authorize_task_read(state, scope, task_id).await?;
    ensure_can_control(scope, &intent, task_id)?;
    Ok(intent)
}

/// Read access to a Sandbox: its own scope/owner pair, or the public visibility
/// its Task grants — a public Task is collaborative, and its Sandbox view is
/// part of what it shares.
///
/// Refuses as `NotFound` so a caller cannot distinguish "not yours" from "does
/// not exist". A missing identity means the request never passed
/// `auth_middleware`, which fails closed as `Forbidden`.
fn ensure_can_read(
    scope: &Option<ScopeContext>,
    intent: &TaskIntent,
    task_id: &str,
) -> Result<(), SandboxError> {
    let ctx = scope
        .as_ref()
        .ok_or_else(|| SandboxError::Forbidden("authentication required".into()))?;
    if intent.visibility == Visibility::Public || ctx.can_read(&intent.scoped_resource()) {
        return Ok(());
    }
    Err(SandboxError::NotFound(format!("task {task_id}")))
}

/// Control access to a Sandbox. Visibility never grants it.
fn ensure_can_control(
    scope: &Option<ScopeContext>,
    intent: &TaskIntent,
    task_id: &str,
) -> Result<(), SandboxError> {
    let ctx = scope
        .as_ref()
        .ok_or_else(|| SandboxError::Forbidden("authentication required".into()))?;
    if ctx.can_control(&intent.scoped_resource()) {
        return Ok(());
    }
    Err(SandboxError::Forbidden(format!(
        "no control access to task {task_id}"
    )))
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

#[cfg(test)]
mod authz_tests {
    //! Unit tests for the per-sandbox authorization gate. The policy itself
    //! lives in `ScopeContext`; these exercise how the handlers compose read +
    //! control over one `TaskIntent`, and which refusal each case earns.
    use super::*;

    use crate::lifecycle::types::ArchiveRetention;
    use moatless_types::access_control::Scope;
    use moatless_types::sandbox::SandboxDesiredState;
    use moatless_types::task::TaskStatus;

    /// Deterministic uuid for a readable test name: distinct names map to
    /// distinct ids, equal names to equal ids.
    fn uid(name: &str) -> uuid::Uuid {
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
    }

    /// A `TaskIntent` carrying the given access-control pair; every other field
    /// is an inert default the access check never reads.
    fn intent(scope: Scope, owner_user_id: Option<&str>, visibility: Visibility) -> TaskIntent {
        TaskIntent {
            status: TaskStatus::Open,
            visibility,
            scope,
            owner_user_id: owner_user_id.map(|u| uid(u).to_string()),
            sandbox_id: "task-1".to_string(),
            workspace_id: "ws-1".to_string(),
            desired_state: SandboxDesiredState::Stopped,
            stop_reason: None,
            retention: None::<ArchiveRetention>,
            marker: None,
            desired_state_updated_at: chrono::Utc::now(),
        }
    }

    fn private(owner_user_id: Option<&str>) -> TaskIntent {
        intent(Scope::User, owner_user_id, Visibility::Private)
    }

    fn ctx(user_id: &str, is_admin: bool) -> ScopeContext {
        ScopeContext {
            user_id: uid(user_id).to_string(),
            is_admin,
        }
    }

    #[test]
    fn read_by_another_tenant_is_not_found() {
        // NotFound, not Forbidden: the refusal never leaks that the sandbox exists.
        let err = ensure_can_read(&Some(ctx("intruder", false)), &private(Some("owner")), "task-1")
            .unwrap_err();
        assert!(matches!(err, SandboxError::NotFound(_)));
    }

    #[test]
    fn read_by_the_owner_is_allowed() {
        assert!(ensure_can_read(&Some(ctx("owner", false)), &private(Some("owner")), "task-1").is_ok());
    }

    #[test]
    fn read_of_a_public_task_is_allowed() {
        // A public Task is collaborative, and its Sandbox view is part of what
        // it shares — read only, never control.
        let public = intent(Scope::User, Some("owner"), Visibility::Public);
        assert!(ensure_can_read(&Some(ctx("stranger", false)), &public, "task-1").is_ok());
        assert!(matches!(
            ensure_can_control(&Some(ctx("stranger", false)), &public, "task-1"),
            Err(SandboxError::Forbidden(_))
        ));
    }

    #[test]
    fn read_of_a_global_sandbox_is_allowed() {
        let global = intent(Scope::Global, None, Visibility::Private);
        assert!(ensure_can_read(&Some(ctx("anyone", false)), &global, "task-1").is_ok());
    }

    #[test]
    fn read_without_identity_is_forbidden() {
        // No identity means the request never passed auth_middleware — fail
        // closed rather than resolving the sandbox under a system context.
        assert!(matches!(
            ensure_can_read(&None, &private(Some("someone")), "task-1"),
            Err(SandboxError::Forbidden(_))
        ));
    }

    #[test]
    fn read_of_an_ownerless_sandbox_is_admin_only() {
        // No Sandbox row leaves the pair ownerless. An admin still reaches it;
        // nobody else does.
        let ownerless = private(None);
        assert!(matches!(
            ensure_can_read(&Some(ctx("anyone", false)), &ownerless, "task-1"),
            Err(SandboxError::NotFound(_))
        ));
        assert!(ensure_can_read(&Some(ctx("root", true)), &ownerless, "task-1").is_ok());
    }

    #[test]
    fn control_by_the_owner_is_allowed() {
        assert!(
            ensure_can_control(&Some(ctx("owner", false)), &private(Some("owner")), "task-1").is_ok()
        );
    }

    #[test]
    fn control_by_a_read_only_viewer_is_forbidden() {
        // Read access never implies control: a global sandbox is admin-controlled.
        let global = intent(Scope::Global, None, Visibility::Private);
        assert!(ensure_can_read(&Some(ctx("viewer", false)), &global, "task-1").is_ok());
        assert!(matches!(
            ensure_can_control(&Some(ctx("viewer", false)), &global, "task-1"),
            Err(SandboxError::Forbidden(_))
        ));
    }

    #[test]
    fn control_by_a_stranger_is_forbidden() {
        let err = ensure_can_control(&Some(ctx("stranger", false)), &private(Some("owner")), "task-1")
            .unwrap_err();
        assert!(matches!(err, SandboxError::Forbidden(_)));
    }

    #[test]
    fn control_by_an_admin_bypasses_scope() {
        assert!(
            ensure_can_control(&Some(ctx("root", true)), &private(Some("other-user")), "task-1").is_ok()
        );
    }

    #[test]
    fn control_without_identity_is_forbidden() {
        assert!(matches!(
            ensure_can_control(&None, &private(Some("someone")), "task-1"),
            Err(SandboxError::Forbidden(_))
        ));
    }
}
