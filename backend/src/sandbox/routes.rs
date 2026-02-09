//! API routes for sandbox operations.
//!
//! Single unified router served on port 8080.
//! Public-facing endpoints are authenticated via Traefik ForwardAuth middleware —
//! the controller itself doesn't validate auth tokens.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use std::sync::Arc;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::provider::SandboxProvider;

/// State shared across sandbox API handlers.
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
        handlers::provision_handler,
        handlers::status_handler,
        handlers::health_check_handler,
        handlers::workspace_status_handler,
        handlers::stop_handler,
        handlers::start_handler,
        handlers::restart_handler,
        handlers::cleanup_handler,
        handlers::server_status_handler,
        handlers::server_logs_handler,
        handlers::search_files_handler,
    )
)]
pub struct SandboxApiDoc;

/// Create the sandbox API router with OpenAPI metadata collection.
pub fn api_router(state: SandboxApiState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(SandboxApiDoc::openapi())
        .routes(routes!(handlers::provision_handler))
        .routes(routes!(handlers::status_handler))
        .routes(routes!(handlers::health_check_handler))
        .routes(routes!(handlers::workspace_status_handler))
        .routes(routes!(handlers::stop_handler))
        .routes(routes!(handlers::start_handler))
        .routes(routes!(handlers::restart_handler))
        .routes(routes!(handlers::cleanup_handler))
        .routes(routes!(handlers::server_status_handler))
        .routes(routes!(handlers::server_logs_handler))
        .routes(routes!(handlers::search_files_handler))
        .with_state(state)
        .split_for_parts();

    let router: axum::Router = Into::<axum::Router>::into(router)
        .route("/health", axum::routing::get(health_handler))
        .route("/healthz", axum::routing::get(health_handler));

    (router, api)
}

/// Health check handler (not documented in OpenAPI — infrastructure only).
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}
