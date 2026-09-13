//! Agent HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use super::error::AgentError;
use super::schemas::ListAgentsResponse;
use super::service::list_agents;
use moatless_core::http_auth::AuthUser;
use moatless_ports::identity::{HarnessCredentials, SecretCatalog};

/// List supported agents with installed / credential status for the current
/// user.
#[utoipa::path(
    get,
    path = "/api/v1/agents",
    tag = "agents",
    responses(
        (status = 200, description = "List of agents", body = ListAgentsResponse)
    )
)]
pub async fn list_agents_handler(
    State(secrets): State<Arc<dyn SecretCatalog>>,
    State(provider_tokens): State<Arc<dyn HarnessCredentials>>,
    auth: AuthUser,
) -> Result<Json<ListAgentsResponse>, AgentError> {
    let ctx = auth.scope_context();
    let agents = list_agents(secrets.as_ref(), provider_tokens.as_ref(), &ctx).await;
    Ok(Json(ListAgentsResponse { agents }))
}
