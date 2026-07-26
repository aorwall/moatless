//! Plugin API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::schemas::{CreatePluginRequest, PluginResponse};
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "plugins", description = "Plugin registry management"),
    ),
    paths(
        handlers::list_plugins,
        handlers::create_plugin,
        handlers::delete_plugin,
        handlers::list_repo_plugins,
        handlers::link_plugin,
        handlers::unlink_plugin,
    ),
    components(
        schemas(CreatePluginRequest, PluginResponse)
    )
)]
pub struct PluginApiDoc;

/// Build the plugin router with OpenAPI metadata.
pub fn plugin_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(PluginApiDoc::openapi())
        .routes(routes!(handlers::list_plugins))
        .routes(routes!(handlers::create_plugin))
        .routes(routes!(handlers::delete_plugin))
        .routes(routes!(handlers::list_repo_plugins))
        .routes(routes!(handlers::link_plugin))
        .routes(routes!(handlers::unlink_plugin))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
