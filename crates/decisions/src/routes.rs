//! Decision route registration and OpenAPI metadata.

use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::handlers;
use crate::state::DecisionsState;

#[derive(OpenApi)]
#[openapi(
    tags((name = "decisions", description = "Multiple-choice decisions from the configured decision provider")),
    paths(handlers::pick_decision)
)]
pub struct DecisionApiDoc;

pub(crate) fn decisions_router(state: DecisionsState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(DecisionApiDoc::openapi())
        .routes(routes!(handlers::pick_decision))
        .with_state(state)
        .split_for_parts();
    (router, api)
}
