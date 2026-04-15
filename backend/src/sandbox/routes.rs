//! API routes for sandbox operations.
//!
//! Single unified router served on port 8080.
//! Public-facing endpoints are authenticated via Traefik ForwardAuth middleware —
//! the controller itself doesn't validate auth tokens.

use axum::Json;
use axum::response::IntoResponse;
use std::sync::Arc;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::file_handlers;
use super::handlers;
use super::provider::SandboxProvider;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::plugin_repo::dao::PluginRepoRepository;
use crate::repository::dao::RepoRepository;
use crate::secret::service::SecretService;
use crate::task::dao::TaskRepository;

/// State shared across sandbox API handlers.
#[derive(Clone)]
pub struct SandboxApiState {
    pub provider: Arc<dyn SandboxProvider>,
    /// Optional task repository — enables server config resolution in handlers.
    pub tasks: Option<Arc<dyn TaskRepository>>,
    /// Optional repo repository — enables server config resolution in handlers.
    pub repos: Option<Arc<dyn RepoRepository>>,
    /// Optional provider token repository — enables credential resolution for provisioning.
    pub provider_tokens: Option<Arc<dyn ProviderTokenRepository>>,
    /// Optional plugin repo repository — enables plugin git URL resolution for provisioning.
    pub plugin_repos: Option<Arc<dyn PluginRepoRepository>>,
    /// Optional scoped secret service — enables server-targeted secret delivery.
    pub secrets: Option<Arc<SecretService>>,
}

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "sandbox", description = "Sandbox lifecycle management"),
    ),
    paths(
        handlers::provision_handler,
        handlers::sandbox_status_handler,
        handlers::stop_handler,
        handlers::restart_handler,
        handlers::redeploy_handler,
        handlers::cleanup_handler,
        handlers::server_status_handler,
        handlers::server_logs_handler,
        file_handlers::search_files_handler,
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
        .routes(routes!(handlers::sandbox_status_handler))
        .routes(routes!(handlers::stop_handler))
        .routes(routes!(handlers::restart_handler))
        .routes(routes!(handlers::redeploy_handler))
        .routes(routes!(handlers::cleanup_handler))
        .routes(routes!(handlers::server_status_handler))
        .routes(routes!(handlers::server_logs_handler))
        .routes(routes!(file_handlers::search_files_handler))
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
