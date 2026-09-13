//! Agent API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::schemas::{AgentDescriptor, AgentModel, ListAgentsResponse};
use moatless_core::http_auth::UserTokenVerifier;
use moatless_ports::identity::{HarnessCredentials, SecretCatalog};

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
///
/// Generic over router state `S` so the handler names its ports (all
/// identity-band) via `FromRef<S>` rather than the whole `AppState`, which the
/// composition root passes as `S`.
pub fn agent_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
    Arc<dyn SecretCatalog>: FromRef<S>,
    Arc<dyn HarnessCredentials>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(AgentApiDoc::openapi())
        .routes(routes!(handlers::list_agents_handler))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
