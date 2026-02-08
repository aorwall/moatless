//! API routes for sandbox operations.
//!
//! Single unified router served on both ports (8080 public, 8081 backward compat).
//! Public-facing endpoints are authenticated via Traefik ForwardAuth middleware —
//! the controller itself doesn't validate auth tokens.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;
use tracing::{error, info};
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::error::SandboxError;
use super::provider::SandboxProvider;
use super::types::{
    ErrorBody, HealthResponse, ProvisionRequest, ProvisionResponse, StatusResponse,
    WorkspaceStatusFullResponse,
};

/// State shared across API handlers.
#[derive(Clone)]
pub struct SandboxApiState {
    pub provider: Arc<dyn SandboxProvider>,
}

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "sandbox", description = "Sandbox workspace lifecycle management"),
    ),
    paths(
        provision_handler,
        status_handler,
        health_check_handler,
        workspace_status_handler,
        stop_handler,
        start_handler,
        restart_handler,
        cleanup_handler,
    )
)]
pub struct SandboxApiDoc;

/// Create the sandbox API router with OpenAPI metadata collection.
pub fn api_router(state: SandboxApiState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(SandboxApiDoc::openapi())
        .routes(routes!(provision_handler))
        .routes(routes!(status_handler))
        .routes(routes!(health_check_handler))
        .routes(routes!(workspace_status_handler))
        .routes(routes!(stop_handler))
        .routes(routes!(start_handler))
        .routes(routes!(restart_handler))
        .routes(routes!(cleanup_handler))
        .with_state(state)
        .split_for_parts();

    // Convert to plain Router and add non-OpenAPI health endpoints
    let router: axum::Router = Into::<axum::Router>::into(router)
        .route("/health", axum::routing::get(health_handler))
        .route("/healthz", axum::routing::get(health_handler));

    (router, api)
}

/// Health check handler (not documented in OpenAPI — infrastructure only).
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Provision a new sandbox workspace for a task.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/provision",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = ProvisionRequest,
    responses(
        (status = 200, description = "Workspace provisioned successfully", body = ProvisionResponse),
        (status = 500, description = "Provisioning failed", body = ErrorBody)
    )
)]
async fn provision_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
    Json(request): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, SandboxError> {
    info!(task_id = %task_id, "POST /provision");
    match state.provider.provision(&task_id, &request).await {
        Ok(response) => {
            info!(task_id = %task_id, "Provision succeeded");
            Ok(Json(response))
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Provision failed");
            Err(e)
        }
    }
}

/// Get environment status for a task workspace.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/env-status",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Environment status", body = StatusResponse),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
async fn status_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state.provider.status(&task_id).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => {
            error!(task_id = %task_id, error = %e, "env-status failed");
            e.into_response()
        }
    }
}

/// Check if a task's sandbox is healthy.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/health",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Health check result", body = HealthResponse)
    )
)]
async fn health_check_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let resp = state.provider.health(&task_id).await;
    Json(resp)
}

/// Get full workspace status including servers and agent state.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/status",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Full workspace status", body = WorkspaceStatusFullResponse),
        (status = 503, description = "Workspace not ready", body = ErrorBody),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
async fn workspace_status_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state.provider.workspace_status(&task_id).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => {
            error!(task_id = %task_id, error = %e, "workspace status failed");
            e.into_response()
        }
    }
}

/// Stop a running workspace.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/stop",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Workspace stopped"),
        (status = 500, description = "Stop failed", body = ErrorBody)
    )
)]
async fn stop_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /stop");
    match state.provider.stop(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Stop succeeded");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Stop failed");
            Err(e)
        }
    }
}

/// Start a stopped workspace.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/start",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Workspace started"),
        (status = 500, description = "Start failed", body = ErrorBody)
    )
)]
async fn start_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /start");
    match state.provider.start(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Start succeeded");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Start failed");
            Err(e)
        }
    }
}

/// Restart a workspace (stop + start).
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/restart",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Workspace restarted"),
        (status = 500, description = "Restart failed", body = ErrorBody)
    )
)]
async fn restart_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /restart");
    match state.provider.restart(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Restart succeeded");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Restart failed");
            Err(e)
        }
    }
}

/// Clean up a workspace and all associated resources.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/cleanup",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Workspace cleaned up"),
        (status = 500, description = "Cleanup failed", body = ErrorBody)
    )
)]
async fn cleanup_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /cleanup");
    match state.provider.cleanup(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Cleanup succeeded");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Cleanup failed");
            Err(e)
        }
    }
}
