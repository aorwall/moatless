//! User settings API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "user-settings", description = "User preferences and settings"),
    ),
    paths(
        handlers::get_settings,
        handlers::update_settings,
    )
)]
pub struct UserSettingsApiDoc;

/// Build the user settings router with OpenAPI metadata.
pub fn user_settings_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(UserSettingsApiDoc::openapi())
        .routes(routes!(handlers::get_settings, handlers::update_settings))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
