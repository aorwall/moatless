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
        handlers::list_providers,
        handlers::slack_webhook,
        handlers::telegram_webhook,
        handlers::list_pairing_requests,
        handlers::approve_pairing,
        handlers::list_my_pairing_requests,
        handlers::webhook_inbound,
        handlers::list_slack_channels,
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
        .routes(routes!(handlers::list_providers))
        .routes(routes!(handlers::slack_webhook))
        .routes(routes!(handlers::telegram_webhook))
        .routes(routes!(handlers::list_pairing_requests))
        .routes(routes!(handlers::approve_pairing))
        .routes(routes!(handlers::list_my_pairing_requests))
        .routes(routes!(handlers::webhook_inbound))
        .routes(routes!(handlers::list_slack_channels))
        .routes(routes!(handlers::create_config, handlers::list_configs))
        .routes(routes!(
            handlers::get_config,
            handlers::update_config,
            handlers::delete_config
        ))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
