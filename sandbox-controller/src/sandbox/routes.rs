//! API routes for sandbox operations.
//!
//! Single unified router served on both ports (8080 public, 8081 backward compat).
//! Public-facing endpoints are authenticated via Traefik ForwardAuth middleware —
//! the controller itself doesn't validate auth tokens.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::sync::Arc;
use tracing::{error, info};

use super::error::SandboxError;
use super::provider::SandboxProvider;
use super::types::{ProvisionRequest, ProvisionResponse};

/// State shared across API handlers.
#[derive(Clone)]
pub struct SandboxApiState {
    pub provider: Arc<dyn SandboxProvider>,
}

/// Create the unified API router (served on both ports).
pub fn api_router(state: SandboxApiState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/healthz", get(health_handler))
        // Unified sandbox API
        .route(
            "/api/sandbox/v1/tasks/{task_id}/provision",
            post(provision_handler),
        )
        .route(
            "/api/sandbox/v1/tasks/{task_id}/env-status",
            get(status_handler),
        )
        .route(
            "/api/sandbox/v1/tasks/{task_id}/health",
            get(health_check_handler),
        )
        .route(
            "/api/sandbox/v1/tasks/{task_id}/status",
            get(workspace_status_handler),
        )
        .route(
            "/api/sandbox/v1/tasks/{task_id}/stop",
            post(stop_handler),
        )
        .route(
            "/api/sandbox/v1/tasks/{task_id}/start",
            post(start_handler),
        )
        .route(
            "/api/sandbox/v1/tasks/{task_id}/restart",
            post(restart_handler),
        )
        .route(
            "/api/sandbox/v1/tasks/{task_id}/cleanup",
            post(cleanup_handler),
        )
        .with_state(state)
}

/// Health check handler.
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// POST /api/sandbox/v1/tasks/{task_id}/provision
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

/// GET /api/sandbox/v1/tasks/{task_id}/env-status
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

/// GET /api/sandbox/v1/tasks/{task_id}/health
async fn health_check_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let resp = state.provider.health(&task_id).await;
    Json(resp)
}

/// GET /api/sandbox/v1/tasks/{task_id}/status
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

/// POST stop
async fn stop_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /stop");
    match state.provider.stop(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Stop succeeded");
            Ok(StatusCode::OK)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Stop failed");
            Err(e)
        }
    }
}

/// POST start
async fn start_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /start");
    match state.provider.start(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Start succeeded");
            Ok(StatusCode::OK)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Start failed");
            Err(e)
        }
    }
}

/// POST restart
async fn restart_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /restart");
    match state.provider.restart(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Restart succeeded");
            Ok(StatusCode::OK)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Restart failed");
            Err(e)
        }
    }
}

/// POST /api/sandbox/v1/tasks/{task_id}/cleanup
async fn cleanup_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /cleanup");
    match state.provider.cleanup(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Cleanup succeeded");
            Ok(StatusCode::OK)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Cleanup failed");
            Err(e)
        }
    }
}
