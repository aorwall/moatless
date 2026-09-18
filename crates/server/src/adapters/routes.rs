//! Canonical adapter API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use axum::{Router, routing::post};

use super::handlers;
use super::providers::{github_pr, linear, slack, telegram, webhook};
use crate::state::AppState;
use moatless_loops::bindings;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "adapters", description = "Adapter connections and webhooks"),
        (name = "task-bindings", description = "Task-to-external-subject bindings"),
        (name = "users", description = "Users"),
        (name = "linear", description = "Linear issue/comment CRUD via workspace OAuth"),
        (name = "github", description = "GitHub PR/issue mutations proxied for sandbox agents"),
    ),
    paths(
        handlers::list_adapters,
        handlers::list_adapter_connections,
        handlers::create_adapter_connection,
        handlers::delete_adapter_connection,
        handlers::adapter_send,
        handlers::send_user_dm,
        bindings::handlers::list_task_bindings,
        bindings::handlers::create_task_binding,
        bindings::handlers::send_task_reply,
        bindings::handlers::reply_to_message,
        bindings::handlers::add_message_reaction,
        bindings::handlers::remove_message_reaction,
        slack::handlers::slack_webhook,
        telegram::handlers::telegram_webhook,
        github_pr::handlers::github_webhook,
        github_pr::api_routes::create_pr,
        github_pr::api_routes::post_comment,
        github_pr::api_routes::reply_to_review_comment,
        github_pr::api_routes::bind_pr,
        github_pr::api_routes::record_external_event,
        linear::handlers::linear_webhook,
        linear::api_routes::get_issue,
        linear::api_routes::create_issue,
        linear::api_routes::list_issues,
        linear::api_routes::update_issue,
        linear::api_routes::create_comment,
        linear::api_routes::mutate_label,
        linear::api_routes::relate_issue,
        linear::api_routes::graphql_passthrough,
        linear::api_routes::lookup,
        webhook::handlers::webhook_inbound,
    )
)]
pub struct AdapterApiDoc;

/// Build the canonical adapter router with OpenAPI metadata.
pub fn adapter_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(AdapterApiDoc::openapi())
        .routes(routes!(handlers::list_adapters))
        .routes(routes!(
            handlers::list_adapter_connections,
            handlers::create_adapter_connection
        ))
        .routes(routes!(handlers::delete_adapter_connection))
        .routes(routes!(handlers::adapter_send))
        .routes(routes!(handlers::send_user_dm))
        .routes(routes!(
            bindings::handlers::list_task_bindings,
            bindings::handlers::create_task_binding
        ))
        .routes(routes!(bindings::handlers::send_task_reply))
        .routes(routes!(bindings::handlers::reply_to_message))
        .routes(routes!(bindings::handlers::add_message_reaction))
        .routes(routes!(bindings::handlers::remove_message_reaction))
        .routes(routes!(slack::handlers::slack_webhook))
        .routes(routes!(telegram::handlers::telegram_webhook))
        .routes(routes!(github_pr::handlers::github_webhook))
        .routes(routes!(github_pr::api_routes::create_pr))
        .routes(routes!(github_pr::api_routes::post_comment))
        .routes(routes!(github_pr::api_routes::reply_to_review_comment))
        .routes(routes!(github_pr::api_routes::bind_pr))
        .routes(routes!(github_pr::api_routes::record_external_event))
        .routes(routes!(linear::handlers::linear_webhook))
        .routes(routes!(linear::api_routes::get_issue))
        .routes(routes!(
            linear::api_routes::create_issue,
            linear::api_routes::list_issues
        ))
        .routes(routes!(linear::api_routes::update_issue))
        .routes(routes!(linear::api_routes::create_comment))
        .routes(routes!(linear::api_routes::mutate_label))
        .routes(routes!(linear::api_routes::relate_issue))
        .routes(routes!(linear::api_routes::graphql_passthrough))
        .routes(routes!(linear::api_routes::lookup))
        .routes(routes!(webhook::handlers::webhook_inbound))
        .with_state(state.clone())
        .split_for_parts();

    let installation = axum::extract::FromRef::from_ref(&state);
    let router = router.merge(moatless_adapter_linear::installation::router(installation));
    (router.merge(legacy_webhook_router(state)), api)
}

fn legacy_webhook_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/v1/channels/slack/webhook",
            post(slack::handlers::slack_webhook),
        )
        .route(
            "/api/v1/channels/telegram/webhook",
            post(telegram::handlers::telegram_webhook),
        )
        .route(
            "/api/v1/channels/github/webhook",
            post(github_pr::handlers::github_webhook),
        )
        .route(
            "/api/v1/channels/webhook/{loop_id}",
            post(webhook::handlers::webhook_inbound),
        )
        .with_state(state)
}
