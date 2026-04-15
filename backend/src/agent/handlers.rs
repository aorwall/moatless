//! Agent HTTP handlers.

use axum::extract::State;
use axum::Json;

use super::error::AgentError;
use super::schemas::ListAgentsResponse;
use super::service::list_agents;
use crate::extractors::AuthUser;
use crate::state::AppState;

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
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<ListAgentsResponse>, AgentError> {
    let ctx = auth.scope_context();
    let agents = list_agents(&state, &ctx).await;
    Ok(Json(ListAgentsResponse { agents }))
}
