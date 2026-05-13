//! Canonical adapter API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use axum::{routing::post, Router};

use super::{github_pr, handlers, linear, slack, telegram, webhook};
use crate::state::AppState;
use crate::{bindings, triggers};

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "adapters", description = "Adapter connections and webhooks"),
        (name = "triggers", description = "Work triggers"),
        (name = "task-bindings", description = "Task-to-external-subject bindings"),
        (name = "users", description = "Users"),
    ),
    paths(
        handlers::list_adapters,
        handlers::list_adapter_connections,
        handlers::create_adapter_connection,
        handlers::adapter_send,
        handlers::send_user_dm,
        triggers::handlers::list_triggers,
        triggers::handlers::create_trigger,
        triggers::handlers::get_trigger,
        triggers::handlers::update_trigger,
        triggers::handlers::delete_trigger,
        triggers::handlers::list_trigger_recent_tasks,
        bindings::handlers::list_task_bindings,
        bindings::handlers::create_task_binding,
        bindings::handlers::send_task_reply,
        bindings::handlers::reply_to_message,
        bindings::handlers::add_message_reaction,
        bindings::handlers::remove_message_reaction,
        slack::handlers::slack_webhook,
        slack::handlers::slack_slash_command,
        telegram::handlers::telegram_webhook,
        github_pr::handlers::github_webhook,
        linear::handlers::linear_webhook,
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
        .routes(routes!(handlers::adapter_send))
        .routes(routes!(handlers::send_user_dm))
        .routes(routes!(
            triggers::handlers::list_triggers,
            triggers::handlers::create_trigger
        ))
        .routes(routes!(triggers::handlers::get_trigger))
        .routes(routes!(
            triggers::handlers::update_trigger,
            triggers::handlers::delete_trigger
        ))
        .routes(routes!(triggers::handlers::list_trigger_recent_tasks))
        .routes(routes!(
            bindings::handlers::list_task_bindings,
            bindings::handlers::create_task_binding
        ))
        .routes(routes!(bindings::handlers::send_task_reply))
        .routes(routes!(bindings::handlers::reply_to_message))
        .routes(routes!(bindings::handlers::add_message_reaction))
        .routes(routes!(bindings::handlers::remove_message_reaction))
        .routes(routes!(slack::handlers::slack_webhook))
        .routes(routes!(slack::handlers::slack_slash_command))
        .routes(routes!(telegram::handlers::telegram_webhook))
        .routes(routes!(github_pr::handlers::github_webhook))
        .routes(routes!(linear::handlers::linear_webhook))
        .routes(routes!(webhook::handlers::webhook_inbound))
        .with_state(state.clone())
        .split_for_parts();

    (router.merge(legacy_webhook_aliases(state)), api)
}

fn legacy_webhook_aliases(state: AppState) -> Router {
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
            "/api/v1/channels/webhook/{trigger_id}",
            post(webhook::handlers::webhook_inbound),
        )
        .with_state(state)
}
