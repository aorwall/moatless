//! Plugin API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::schemas::{
    ActivationReach, ActivationResponse, CreatePluginRequest, EffectivePluginResponse,
    PluginResponse, PluginSkillResponse, SetActivationRequest,
};
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
        handlers::list_plugin_skills,
        handlers::list_plugin_activations,
        handlers::set_plugin_activation,
        handlers::clear_plugin_activation,
        handlers::list_effective_plugins,
    ),
    components(
        schemas(
            ActivationReach,
            ActivationResponse,
            CreatePluginRequest,
            EffectivePluginResponse,
            PluginResponse,
            PluginSkillResponse,
            SetActivationRequest,
        )
    )
)]
pub struct PluginApiDoc;

/// Build the plugin router with OpenAPI metadata.
pub fn plugin_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(PluginApiDoc::openapi())
        .routes(routes!(handlers::list_plugins))
        .routes(routes!(handlers::create_plugin))
        .routes(routes!(handlers::delete_plugin))
        .routes(routes!(handlers::list_plugin_skills))
        .routes(routes!(handlers::list_plugin_activations))
        .routes(routes!(
            handlers::set_plugin_activation,
            handlers::clear_plugin_activation
        ))
        .routes(routes!(handlers::list_effective_plugins))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
