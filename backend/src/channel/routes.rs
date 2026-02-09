//! Channel API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "channels", description = "Channel integration management and webhooks"),
    ),
    paths(
        handlers::slack_webhook,
        handlers::create_config,
        handlers::list_configs,
        handlers::get_config,
        handlers::update_config,
        handlers::delete_config,
    )
)]
pub struct ChannelApiDoc;

/// Build the channel router with OpenAPI metadata.
pub fn channel_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(ChannelApiDoc::openapi())
        .routes(routes!(handlers::slack_webhook))
        .routes(routes!(handlers::create_config, handlers::list_configs))
        .routes(routes!(
            handlers::get_config,
            handlers::update_config,
            handlers::delete_config
        ))
        .with_state(state)
        .split_for_parts();

    (router.into(), api)
}
