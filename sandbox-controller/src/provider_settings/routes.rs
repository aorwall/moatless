//! Provider settings API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "provider-settings", description = "GitHub OAuth/PAT management"),
    ),
    paths(
        handlers::get_github_config,
        handlers::save_github_pat,
        handlers::get_github_oauth_url,
        handlers::exchange_github_token,
        handlers::delete_github_config,
    )
)]
pub struct ProviderSettingsApiDoc;

/// Build the provider settings router with OpenAPI metadata.
pub fn provider_settings_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(ProviderSettingsApiDoc::openapi())
        .routes(routes!(handlers::get_github_config, handlers::delete_github_config))
        .routes(routes!(handlers::save_github_pat))
        .routes(routes!(handlers::get_github_oauth_url))
        .routes(routes!(handlers::exchange_github_token))
        .with_state(state)
        .split_for_parts();

    (router.into(), api)
}
