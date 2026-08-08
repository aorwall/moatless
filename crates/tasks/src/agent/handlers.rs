//! Agent HTTP handlers.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;

use super::error::AgentError;
use super::schemas::ListAgentsResponse;
use super::service::list_agents;
use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
use moatless_identity::extractors::AuthUser;
use moatless_identity::secret::dao::SecretDao;

/// List supported agents with installed / credential status for the current user.
#[utoipa::path(
    get,
    path = "/api/v1/agents",
    tag = "agents",
    responses(
        (status = 200, description = "List of agents", body = ListAgentsResponse)
    )
)]
pub async fn list_agents_handler(
    State(secret_records): State<Arc<dyn SecretDao>>,
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
) -> Result<Json<ListAgentsResponse>, AgentError> {
    let ctx = auth.scope_context();
    let agents = list_agents(secret_records.as_ref(), provider_tokens.as_ref(), &ctx).await;
    Ok(Json(ListAgentsResponse { agents }))
}
