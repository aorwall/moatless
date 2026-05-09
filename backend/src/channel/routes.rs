//! Canonical integration API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::{github_pr, handlers, linear, slack, telegram, webhook};
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "integrations", description = "Provider connections and webhooks"),
        (name = "task-routes", description = "Task routing policies"),
        (name = "triggers", description = "External event triggers"),
        (name = "scheduled-jobs", description = "Durable scheduled task triggers"),
        (name = "task-bindings", description = "Task-to-external-subject bindings"),
    ),
    paths(
        handlers::connections::list_integration_providers,
        handlers::connections::list_provider_connections,
        handlers::connections::create_provider_connection,
        handlers::routes::list_task_routes,
        handlers::routes::create_task_route,
        handlers::routes::update_task_route,
        handlers::triggers::list_task_triggers,
        handlers::triggers::create_task_trigger,
        handlers::triggers::update_task_trigger,
        handlers::triggers::delete_task_trigger,
        handlers::scheduled_jobs::list_scheduled_jobs,
        handlers::scheduled_jobs::create_scheduled_job,
        handlers::scheduled_jobs::update_scheduled_job,
        handlers::bindings::list_task_bindings,
        handlers::bindings::create_task_binding,
        handlers::bindings::send_task_reply,
        slack::handlers::slack_webhook,
        slack::handlers::slack_slash_command,
        telegram::handlers::telegram_webhook,
        github_pr::handlers::github_webhook,
        linear::handlers::linear_webhook,
        webhook::handlers::webhook_inbound,
    )
)]
pub struct ChannelApiDoc;

/// Build the canonical integration router with OpenAPI metadata.
pub fn channel_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(ChannelApiDoc::openapi())
        .routes(routes!(handlers::connections::list_integration_providers))
        .routes(routes!(
            handlers::connections::list_provider_connections,
            handlers::connections::create_provider_connection
        ))
        .routes(routes!(
            handlers::routes::list_task_routes,
            handlers::routes::create_task_route
        ))
        .routes(routes!(handlers::routes::update_task_route))
        .routes(routes!(
            handlers::triggers::list_task_triggers,
            handlers::triggers::create_task_trigger
        ))
        .routes(routes!(
            handlers::triggers::update_task_trigger,
            handlers::triggers::delete_task_trigger
        ))
        .routes(routes!(
            handlers::scheduled_jobs::list_scheduled_jobs,
            handlers::scheduled_jobs::create_scheduled_job
        ))
        .routes(routes!(handlers::scheduled_jobs::update_scheduled_job))
        .routes(routes!(
            handlers::bindings::list_task_bindings,
            handlers::bindings::create_task_binding
        ))
        .routes(routes!(handlers::bindings::send_task_reply))
        .routes(routes!(slack::handlers::slack_webhook))
        .routes(routes!(slack::handlers::slack_slash_command))
        .routes(routes!(telegram::handlers::telegram_webhook))
        .routes(routes!(github_pr::handlers::github_webhook))
        .routes(routes!(linear::handlers::linear_webhook))
        .routes(routes!(webhook::handlers::webhook_inbound))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
