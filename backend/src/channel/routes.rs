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
        handlers::slack_slash_command,
        handlers::telegram_webhook,
        handlers::github_pr_webhook,
        handlers::linear_webhook,
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
        handlers::trigger_channel,
        handlers::direct_send_to_channel,
        handlers::direct_send_to_rule,
        handlers::connect_channel,
        handlers::get_defaults,
        handlers::set_defaults,
        handlers::list_task_channels,
        handlers::send_to_task_channels,
        handlers::send_to_task_channel,
        handlers::send_to_task_channel_thread,
        handlers::send_to_task_channel_subscription,
        handlers::create_task_subscription,
        handlers::create_ingress_rule,
        handlers::list_ingress_rules,
        handlers::delete_ingress_rule,
        handlers::list_provider_connections,
        handlers::create_provider_connection,
        handlers::send_user_dm_handler,
        handlers::send_to_chat_destination,
    )
)]
pub struct ChannelApiDoc;

/// Build the channel router with OpenAPI metadata.
pub fn channel_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(ChannelApiDoc::openapi())
        .routes(routes!(handlers::list_providers))
        .routes(routes!(handlers::slack_webhook))
        .routes(routes!(handlers::slack_slash_command))
        .routes(routes!(handlers::telegram_webhook))
        .routes(routes!(handlers::github_pr_webhook))
        .routes(routes!(handlers::linear_webhook))
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
        .routes(routes!(handlers::trigger_channel))
        .routes(routes!(handlers::direct_send_to_channel))
        .routes(routes!(handlers::direct_send_to_rule))
        .routes(routes!(handlers::connect_channel))
        .routes(routes!(handlers::get_defaults, handlers::set_defaults))
        .routes(routes!(handlers::list_task_channels))
        .routes(routes!(handlers::send_to_task_channels))
        .routes(routes!(handlers::send_to_task_channel))
        .routes(routes!(handlers::send_to_task_channel_thread))
        .routes(routes!(handlers::send_to_task_channel_subscription))
        .routes(routes!(handlers::create_task_subscription))
        .routes(routes!(
            handlers::create_ingress_rule,
            handlers::list_ingress_rules
        ))
        .routes(routes!(handlers::delete_ingress_rule))
        .routes(routes!(
            handlers::list_provider_connections,
            handlers::create_provider_connection
        ))
        .routes(routes!(handlers::send_user_dm_handler))
        .routes(routes!(handlers::send_to_chat_destination))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
