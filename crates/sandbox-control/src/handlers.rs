//! Sandbox handler functions.
//!
//! Extracted from routes.rs for consistency with other modules (task/,
//! message/, agent/, etc.).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{Stream, StreamExt};
use serde::Deserialize;
use tracing::{error, info, warn};

use moatless_api_schemas::sandbox::WorkspaceRepositoryAccessResponse;
use moatless_sandbox_runtime::access::LogOptions;
use moatless_sandbox_runtime::error::SandboxError;
use moatless_sandbox_runtime::live_status::apply_running_rule;
use moatless_types::sandbox::ArchiveRetention;

use crate::workspace_repository::GrantError;

use super::access::{
    authorize_task_owner, authorize_task_read, authorize_task_write, require_lifecycle,
};
use super::state::SandboxApiState;
use moatless_core::http_auth::OptionalScope;
use moatless_sandbox_runtime::schemas::*;
use moatless_types::sandbox::SandboxStopReason;

/// Provision a new sandbox for a task.
///
/// Builds the provision request internally from DB state (task, repository,
/// credentials); no request body required. Requires write access.
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
        (status = 403, description = "No write access to the task", body = moatless_api_schemas::error::ErrorBody),
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
    authorize_task_write(&state, &scope, &task_id).await?;

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
        (status = 200, description = "Task live status", body = moatless_sandbox_runtime::live_status::LiveStatus),
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

    let status = match state
        .observer
        .live_status(&task_id, intent.status, intent.desired_state)
        .await
    {
        Ok(status) => status,
        Err(e) => return e.into_response(),
    };

    // The runtime saw the pod and the heartbeat; turn rows are the other
    // half of the same question. Routing both through `derive_running` is
    // what keeps this surface and the conversation list agreeing on whether
    // an agent is working. A failed turn read degrades to the runtime's own
    // answer rather than an error: 500s here is worse than answering alone.
    let turns = match state.turns.as_deref() {
        Some(reader) => match reader.turn_signals(&task_id).await {
            Ok(signals) => Some(signals),
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "live status: turn signals unavailable");
                None
            }
        },
        None => None,
    };

    let status = match turns {
        Some(turns) => apply_running_rule(status, intent.desired_state, intent.stop_reason, turns),
        None => status,
    };

    Json(status).into_response()
}

/// Get full sandbox status including Agent status.
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
            resp.sandbox_desired_state = Some(desired_state);

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
    authorize_task_write(&state, &scope, &task_id).await?;
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
    authorize_task_write(&state, &scope, &task_id).await?;
    // lifecycle.restart detaches internally, so it runs to completion even if the
    // HTTP client disconnects.
    require_lifecycle(&state)?.restart(&task_id).await?;
    info!(task_id = %task_id, "Restart succeeded");
    Ok(StatusCode::NO_CONTENT)
}

/// Redeploy a sandbox (delete Deployment/Service/IngressRoutes, recreate with
/// fresh repo config).
///
/// Preserves workspace data (PVC). Use after changing the workspace configuration.
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
    authorize_task_write(&state, &scope, &task_id).await?;

    // Lifecycle owns plan construction and detaches internally; the runtime
    // receives a fresh request from DB state.
    let outcome = require_lifecycle(&state)?.redeploy(&task_id).await?;
    info!(task_id = %task_id, "Redeploy succeeded");
    Ok(Json(ProvisionResponse {
        status: outcome.observed.sandbox_status(),
    }))
}

/// Mint direct access to a task's workspace `restic` repository.
///
/// Platform admins only, on top of ownership — ownership alone is not enough:
/// this hands out the S3 session and repository password, everything the
/// `restic` CLI (and an attacker) would need.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/workspace-repository/access",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Repository access granted", body = moatless_api_schemas::sandbox::WorkspaceRepositoryAccessResponse),
        (status = 403, description = "Not a platform admin", body = moatless_api_schemas::error::ErrorBody),
        (status = 503, description = "Workspace repositories are not configured", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[tracing::instrument(skip_all, fields(task_id = %task_id))]
pub async fn workspace_repository_access_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
) -> Result<Json<WorkspaceRepositoryAccessResponse>, SandboxError> {
    authorize_task_owner(&state, &scope, &task_id).await?;
    if !scope.as_ref().is_some_and(|ctx| ctx.is_admin) {
        return Err(SandboxError::Forbidden(
            "workspace repository access is limited to platform admins".into(),
        ));
    }

    // `NotReady`, not `ConfigMissing`: the latter renders a bare 500, so an
    // admin on a deployment that simply has no role ARN would see nothing that
    // says so.
    let access = state.workspace_repository.as_deref().ok_or_else(|| {
        SandboxError::NotReady("workspace repositories are not configured".into())
    })?;
    // `grant` logs the cause. No fork source: an admin opening the CLI asked
    // for this task's repository, and a source would only widen what those
    // credentials reach.
    let grant = access
        .grant(&task_id, None)
        .await
        .map_err(|error| match error {
            GrantError::Sts(_) => {
                SandboxError::Internal("Failed to grant workspace repository access".into())
            }
        })?;

    info!(task_id = %task_id, expires_at = %grant.expires_at, "Granted workspace repository access");
    Ok(Json(WorkspaceRepositoryAccessResponse {
        repository: grant.restic_url(),
        bucket: grant.bucket,
        prefix: grant.prefix,
        region: grant.region,
        access_key_id: grant.access_key_id,
        secret_access_key: grant.secret_access_key,
        session_token: grant.session_token,
        expires_at: grant
            .expires_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        password: grant.password,
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
    authorize_task_owner(&state, &scope, &task_id).await?;
    // A cleanup deletes archives (no retention) — lifecycle owns the Removed
    // intent and strict teardown.
    require_lifecycle(&state)?
        .remove(&task_id, ArchiveRetention::DeleteArchives)
        .await?;
    info!(task_id = %task_id, "Cleanup succeeded");
    Ok(StatusCode::NO_CONTENT)
}

/// Query parameters for the sandbox logs endpoint.
#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    /// Fetch logs from the previous container instance (useful for crashed
    /// containers).
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

/// Stream the sandbox container's logs via SSE.
///
/// `?previous=true` fetches logs from a crashed/restarted container;
/// `?follow=false` ends the stream after the buffered lines instead of
/// waiting for more. OpenAPI cannot represent an SSE response body.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/logs",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("previous" = bool, Query, description = "Fetch logs from previous container instance"),
        ("follow" = Option<bool>, Query, description = "Keep the stream open for new lines (default true)"),
        ("tail" = Option<i64>, Query, description = "Number of trailing lines to start from (default 500)")
    ),
    responses(
        (status = 200, description = "SSE stream of sandbox log lines"),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn sandbox_logs_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, SandboxError>>>, SandboxError> {
    authorize_task_read(&state, &scope, &task_id).await?;
    let lines = state
        .observer
        .sandbox_logs(
            &task_id,
            LogOptions {
                previous: query.previous,
                follow: query.follow,
                tail_lines: query.tail,
            },
        )
        .await?;
    let events = lines.map(|line| line.map(|line| Event::default().data(line)));
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

/// Health check handler (not documented in OpenAPI — infrastructure only).
pub(super) async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}
