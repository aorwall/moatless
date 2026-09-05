//! API routes for sandbox operations.
//!
//! Single unified router served on port 8080.
//!
//! These endpoints run behind the app-level `auth_middleware` (session cookie
//! or API key), which injects an `AuthIdentity` extension. Each handler then
//! authorizes the caller against the target Sandbox via the `OptionalScope`
//! extractor and the `authorize_task_read`/`authorize_task_write` helpers, so
//! a caller can only read or operate a sandbox they may access.

use axum::Json;
use axum::response::IntoResponse;
use std::sync::Arc;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::file_handlers;
use super::handlers;
use super::lifecycle::Lifecycle;
use moatless_ports::sandbox::WorkspaceSnapshotSource;
use moatless_ports::task::TurnStateReader;
use moatless_sandbox_runtime::client::SandboxConnector;
use moatless_sandbox_runtime::status_reader::SandboxStatusReader;

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
    /// The Task band's half of the running rule. `live_status` folds it into
    /// the runtime's answer so this surface and the T3 conversation list decide
    /// "is the agent working" from the same rule. `None` in the no-database
    /// mode, where there are no turn rows to read and the raw heartbeat is all
    /// there is.
    pub turns: Option<Arc<dyn TurnStateReader>>,
    /// The workspace as S3 last stored it, for reading a Task's files while no
    /// sandbox is running. `None` when the deployment stores no snapshots (no S3
    /// bucket configured), in which case a stopped Task answers as before —
    /// sandbox not ready.
    pub snapshots: Option<Arc<dyn WorkspaceSnapshotSource>>,
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
