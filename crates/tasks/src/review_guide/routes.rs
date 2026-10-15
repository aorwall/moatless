//! Guided-review API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::operations::ReviewGuides;
use crate::task::operations::TaskCommands;
use moatless_core::http_auth::UserTokenVerifier;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "review-guides", description = "Guided pull request reviews written by an agent"),
    ),
    paths(
        handlers::get_review_guide_run,
        handlers::submit_review_guide,
    )
)]
pub struct ReviewGuideApiDoc;

/// Build the guided-review router with OpenAPI metadata, generic over router
/// state `S` like [`crate::feedback::routes::feedback_router`].
pub fn review_guide_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<ReviewGuides>: FromRef<S>,
    Arc<TaskCommands>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    OpenApiRouter::with_openapi(ReviewGuideApiDoc::openapi())
        .routes(routes!(
            handlers::get_review_guide_run,
            handlers::submit_review_guide
        ))
        .with_state(state)
        .split_for_parts()
}
