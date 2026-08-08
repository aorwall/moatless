//! Agent API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::schemas::{AgentDescriptor, AgentModel, ListAgentsResponse};
use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
use moatless_identity::extractors::UserTokenVerifier;
use moatless_identity::secret::dao::SecretDao;

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
/// Generic over the router state `S`: the handler extracts its ports through
/// `FromRef<S>`, so this router names those ports (all identity-band) rather
/// than the whole `AppState`. The composition root passes `AppState` as `S`.
pub fn agent_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
    Arc<dyn SecretDao>: FromRef<S>,
    Arc<dyn ProviderTokenDao>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(AgentApiDoc::openapi())
        .routes(routes!(handlers::list_agents_handler))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
