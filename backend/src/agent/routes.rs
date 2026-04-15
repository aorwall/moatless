//! Agent API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::schemas::{AgentDescriptor, AgentModel, ListAgentsResponse};
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "agents", description = "Agent catalog and availability"),
    ),
    paths(handlers::list_agents_handler),
    components(schemas(AgentDescriptor, AgentModel, ListAgentsResponse))
)]
pub struct AgentApiDoc;

/// Build the agent router with OpenAPI metadata.
pub fn agent_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(AgentApiDoc::openapi())
        .routes(routes!(handlers::list_agents_handler))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
