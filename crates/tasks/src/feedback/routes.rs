//! Feedback API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::dao::FeedbackDao;
use super::handlers;
use moatless_core::http_auth::UserTokenVerifier;
use crate::task::dao::TaskDao;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "feedback", description = "Qualitative feedback on agent runs"),
    ),
    paths(
        handlers::create_feedback,
        handlers::list_task_feedback,
    )
)]
pub struct FeedbackApiDoc;

/// Build the feedback router with OpenAPI metadata.
///
/// Generic over the router state `S` so this module names no root type: the
/// composition root supplies the concrete state at the call site, and each
/// handler reaches its ports — `Arc<dyn FeedbackDao>`, `Arc<dyn TaskDao>`, and
/// the `UserTokenVerifier` behind `AuthUser` — through `FromRef<S>`.
pub fn feedback_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn FeedbackDao>: FromRef<S>,
    Arc<dyn TaskDao>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(FeedbackApiDoc::openapi())
        .routes(routes!(handlers::create_feedback))
        .routes(routes!(handlers::list_task_feedback))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
