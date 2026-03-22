//! Plugin repo API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "plugin-repos", description = "Plugin repository management"),
    ),
    paths(
        handlers::list_plugin_repos,
        handlers::create_plugin_repo,
        handlers::delete_plugin_repo,
        handlers::list_repo_plugins,
        handlers::link_plugin,
        handlers::unlink_plugin,
    )
)]
pub struct PluginRepoApiDoc;

/// Build the plugin repo router with OpenAPI metadata.
pub fn plugin_repo_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(PluginRepoApiDoc::openapi())
        .routes(routes!(handlers::list_plugin_repos))
        .routes(routes!(handlers::create_plugin_repo))
        .routes(routes!(handlers::delete_plugin_repo))
        .routes(routes!(handlers::list_repo_plugins))
        .routes(routes!(handlers::link_plugin))
        .routes(routes!(handlers::unlink_plugin))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
