//! HTTP handlers for decisions.

use axum::Json;
use axum::extract::State;
use moatless_api_schemas::decisions::{PickDecisionRequest, PickDecisionResponse};
use moatless_api_schemas::error::ErrorBody;
use moatless_core::http_auth::AuthUser;

use crate::error::DecisionError;
use crate::operations::{self, Caller};
use crate::state::DecisionsState;

/// Pick one option for a piece of text with the configured decision provider.
#[utoipa::path(
    post,
    path = "/api/v1/decisions/pick",
    tag = "decisions",
    request_body = PickDecisionRequest,
    responses(
        (status = 200, description = "The picked option and its band", body = PickDecisionResponse),
        (status = 400, description = "Invalid question, options or state, or rejected by the provider", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 502, description = "The decision provider failed", body = ErrorBody),
        (status = 503, description = "No decision provider is configured, or it is overloaded", body = ErrorBody)
    )
)]
pub(crate) async fn pick_decision(
    State(state): State<DecisionsState>,
    auth: AuthUser,
    Json(req): Json<PickDecisionRequest>,
) -> Result<Json<PickDecisionResponse>, DecisionError> {
    let Some(decider) = state.decider.as_deref() else {
        let error = DecisionError::Unconfigured;
        crate::metrics::record_unconfigured(error.outcome());
        return Err(error);
    };
    let caller = Caller {
        user_id: auth.user_id,
        task_id: req.task_id.as_deref().or(auth.task_id.as_deref()),
    };
    operations::pick(decider, caller, &req).await.map(Json)
}
