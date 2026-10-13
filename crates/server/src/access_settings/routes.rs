//! Access administration routes and OpenAPI tags.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "git-host-access", description = "GitHub and Forgejo provider-token access"),
        (name = "agent-harness-access", description = "Agent harness credential access"),
        (name = "app-administration", description = "Adapter, GitHub App and Forgejo instance administration"),
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
        handlers::admin_list_provider_apps,
        handlers::admin_save_provider_app,
        handlers::admin_remove_provider_app,
        handlers::admin_list_adapter_apps,
        handlers::admin_save_adapter_app,
        handlers::admin_delete_adapter_app,
        handlers::get_forgejo_config,
        handlers::list_forgejo_connections,
        handlers::save_forgejo_pat,
        handlers::delete_forgejo_pat_override,
        handlers::delete_forgejo_config,
        handlers::admin_list_forgejo_instances,
        handlers::admin_register_forgejo_instance,
        handlers::admin_remove_forgejo_instance,
        handlers::get_codex_config,
        handlers::save_codex_auth,
        handlers::delete_codex_config,
        handlers::admin_save_global_codex_auth,
        handlers::admin_delete_global_codex_config,
        handlers::start_codex_device_login,
        handlers::poll_codex_device_login,
        handlers::admin_set_codex_provider_token_handler,
    )
)]
pub struct AccessSettingsApiDoc;

pub fn access_settings_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(AccessSettingsApiDoc::openapi())
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
        .routes(routes!(handlers::admin_list_provider_apps))
        .routes(routes!(
            handlers::admin_save_provider_app,
            handlers::admin_remove_provider_app
        ))
        .routes(routes!(handlers::admin_list_adapter_apps))
        .routes(routes!(
            handlers::admin_save_adapter_app,
            handlers::admin_delete_adapter_app
        ))
        .routes(routes!(
            handlers::get_forgejo_config,
            handlers::delete_forgejo_config
        ))
        .routes(routes!(handlers::list_forgejo_connections))
        .routes(routes!(
            handlers::save_forgejo_pat,
            handlers::delete_forgejo_pat_override
        ))
        .routes(routes!(handlers::admin_list_forgejo_instances))
        .routes(routes!(
            handlers::admin_register_forgejo_instance,
            handlers::admin_remove_forgejo_instance
        ))
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
        .routes(routes!(handlers::admin_set_codex_provider_token_handler))
        .with_state(state)
        .split_for_parts();
    (router, api)
}
