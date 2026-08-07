//! Plugin API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::dao::PluginDao;
use super::handlers;
use super::schemas::{
    ActivationReach, ActivationResponse, CreatePluginRequest, EffectivePluginResponse,
    PluginResponse, PluginSkillResponse, SetActivationRequest,
};
use super::sync::PluginSyncService;
use moatless_identity::extractors::UserTokenVerifier;

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
///
/// Generic over the router state `S` so this module names no root type: the
/// composition root supplies the concrete state at the call site, and each
/// handler reaches its ports — `Arc<dyn PluginDao>`, `Arc<PluginSyncService>`,
/// and the `UserTokenVerifier` behind `AuthUser` — through `FromRef<S>`.
pub fn plugin_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn PluginDao>: FromRef<S>,
    Arc<PluginSyncService>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
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
