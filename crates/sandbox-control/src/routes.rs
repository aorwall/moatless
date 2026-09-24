//! Sandbox HTTP route registration.
//!
//! Single unified router served on port 8080.
//!
//! These endpoints run behind the app-level `auth_middleware` (session cookie
//! or API key), which injects an `AuthIdentity` extension; each handler then
//! authorizes the caller against the target Sandbox via `OptionalScope` and the
//! `access` operations, so a caller can only read or operate a sandbox they may
//! access.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::file_handlers;
use super::handlers;

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
        handlers::sandbox_logs_handler,
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
        .routes(routes!(handlers::workspace_repository_access_handler))
        .routes(routes!(handlers::sandbox_logs_handler))
        .routes(routes!(file_handlers::file_tree_handler))
        .routes(routes!(file_handlers::read_file_handler))
        .routes(routes!(file_handlers::write_file_handler))
        .routes(routes!(file_handlers::list_files_handler))
        .routes(routes!(file_handlers::search_by_name_handler))
        .with_state(state)
        .split_for_parts();

    let router: axum::Router = Into::<axum::Router>::into(router)
        .route("/health", axum::routing::get(handlers::health_handler))
        .route("/healthz", axum::routing::get(handlers::health_handler));

    (router, api)
}

pub use super::state::SandboxApiState;
