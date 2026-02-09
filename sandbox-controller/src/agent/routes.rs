//! Agent API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "agent", description = "Agent execution — message sending, commands, and lifecycle"),
    ),
    paths(
        handlers::send_message,
        handlers::send_command,
        handlers::stop_agent,
    )
)]
pub struct AgentApiDoc;

/// Build the agent router with OpenAPI metadata.
pub fn agent_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(AgentApiDoc::openapi())
        .routes(routes!(handlers::send_message))
        .routes(routes!(handlers::send_command))
        .routes(routes!(handlers::stop_agent))
        .with_state(state)
        .split_for_parts();

    (router.into(), api)
}
