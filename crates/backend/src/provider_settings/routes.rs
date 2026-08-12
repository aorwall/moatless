//! Provider settings API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "provider-settings", description = "GitHub, Gitness, and Codex credential management"),
    ),
    paths(
        handlers::get_github_config,
        handlers::save_github_pat,
        handlers::delete_github_pat_override,
        handlers::delete_github_config,
        handlers::admin_list_github_apps,
        handlers::admin_register_github_app,
        handlers::admin_rotate_github_app_key,
        handlers::admin_remove_github_app,
        handlers::admin_list_adapter_apps,
        handlers::admin_save_adapter_app,
        handlers::admin_delete_adapter_app,
        handlers::admin_get_global_github_installation,
        handlers::admin_put_global_github_installation,
        handlers::admin_delete_global_github_installation,
        handlers::get_gitness_config,
        handlers::save_gitness_pat,
        handlers::delete_gitness_config,
        handlers::get_codex_config,
        handlers::save_codex_auth,
        handlers::delete_codex_config,
        handlers::admin_save_global_codex_auth,
        handlers::admin_delete_global_codex_config,
        handlers::start_codex_device_login,
        handlers::poll_codex_device_login,
    )
)]
pub struct ProviderSettingsApiDoc;

/// Build the provider settings router with OpenAPI metadata.
pub fn provider_settings_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(ProviderSettingsApiDoc::openapi())
        .routes(routes!(
            handlers::get_github_config,
            handlers::delete_github_config
        ))
        .routes(routes!(
            handlers::save_github_pat,
            handlers::delete_github_pat_override
        ))
        .routes(routes!(
            handlers::admin_list_github_apps,
            handlers::admin_register_github_app
        ))
        .routes(routes!(handlers::admin_remove_github_app))
        .routes(routes!(handlers::admin_rotate_github_app_key))
        .routes(routes!(handlers::admin_list_adapter_apps))
        .routes(routes!(
            handlers::admin_save_adapter_app,
            handlers::admin_delete_adapter_app
        ))
        .routes(routes!(
            handlers::admin_get_global_github_installation,
            handlers::admin_put_global_github_installation,
            handlers::admin_delete_global_github_installation
        ))
        .routes(routes!(
            handlers::get_gitness_config,
            handlers::delete_gitness_config
        ))
        .routes(routes!(handlers::save_gitness_pat))
        .routes(routes!(
            handlers::get_codex_config,
            handlers::delete_codex_config
        ))
        .routes(routes!(handlers::save_codex_auth))
        .routes(routes!(
            handlers::admin_save_global_codex_auth,
            handlers::admin_delete_global_codex_config
        ))
        .routes(routes!(handlers::start_codex_device_login))
        .routes(routes!(handlers::poll_codex_device_login))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
