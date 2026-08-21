//! API routes for sandbox operations.
//!
//! Single unified router served on port 8080.
//!
//! These endpoints run behind the app-level `auth_middleware` (session cookie
//! or API key), which injects an `AuthIdentity` extension. Each handler then
//! authorizes the caller against the target Sandbox via the `OptionalScope`
//! extractor and the `authorize_task_read`/`authorize_task_control` helpers, so
//! a caller can only read or operate a sandbox they may access.

use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::status_reader::SandboxStatusReader;
use crate::client::SandboxConnector;
use super::file_handlers;
use super::handlers;
use super::lifecycle::Lifecycle;

/// State shared across sandbox API handlers.
#[derive(Clone)]
pub struct SandboxApiState {
    pub observer: Arc<dyn SandboxStatusReader>,
    pub connector: Arc<dyn SandboxConnector>,
    /// Shared durable lifecycle handle — the sole writer of Sandbox intent. The
    /// lifecycle HTTP handlers (`provision`/`stop`/`restart`/`redeploy`/`cleanup`)
    /// drive transitions through it. `None` only in the no-database mode, where
    /// those handlers return a clean "unavailable" error.
    pub lifecycle: Option<Arc<Lifecycle>>,
}

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "sandbox", description = "Sandbox lifecycle management"),
    ),
    paths(
        handlers::provision_handler,
        handlers::live_status_handler,
        handlers::sandbox_status_handler,
        handlers::stop_handler,
        handlers::restart_handler,
        handlers::redeploy_handler,
        handlers::remove_handler,
        handlers::server_status_handler,
        handlers::server_logs_handler,
        handlers::list_server_config_handler,
        handlers::update_server_config_handler,
        handlers::restart_server_handler,
        handlers::reset_server_config_handler,
        file_handlers::file_tree_handler,
        file_handlers::read_file_handler,
        file_handlers::write_file_handler,
        file_handlers::list_files_handler,
        file_handlers::search_by_name_handler,
    )
)]
pub struct SandboxApiDoc;

/// Create the sandbox API router with OpenAPI metadata collection.
pub fn api_router(state: SandboxApiState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(SandboxApiDoc::openapi())
        .routes(routes!(handlers::provision_handler))
        .routes(routes!(handlers::live_status_handler))
        .routes(routes!(handlers::sandbox_status_handler))
        .routes(routes!(handlers::stop_handler))
        .routes(routes!(handlers::restart_handler))
        .routes(routes!(handlers::redeploy_handler))
        .routes(routes!(handlers::remove_handler))
        .routes(routes!(handlers::server_status_handler))
        .routes(routes!(handlers::server_logs_handler))
        .routes(routes!(handlers::list_server_config_handler))
        .routes(routes!(handlers::update_server_config_handler))
        .routes(routes!(handlers::restart_server_handler))
        .routes(routes!(handlers::reset_server_config_handler))
        .routes(routes!(file_handlers::file_tree_handler))
        .routes(routes!(file_handlers::read_file_handler))
        .routes(routes!(file_handlers::write_file_handler))
        .routes(routes!(file_handlers::list_files_handler))
        .routes(routes!(file_handlers::search_by_name_handler))
        .with_state(state)
        .split_for_parts();

    let router: axum::Router = Into::<axum::Router>::into(router)
        .route("/health", axum::routing::get(health_handler))
        .route("/healthz", axum::routing::get(health_handler));

    (router, api)
}

/// Health check handler (not documented in OpenAPI — infrastructure only).
async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}
