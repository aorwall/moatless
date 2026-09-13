//! Feedback API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::operations::FeedbackNotes;
use crate::task::operations::TaskCommands;
use moatless_core::http_auth::UserTokenVerifier;
use moatless_workspace_ports::RepositoryReader;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "feedback", description = "Qualitative feedback on agent runs"),
    ),
    paths(
        handlers::create_feedback,
        handlers::set_feedback_status,
        handlers::list_task_feedback,
        handlers::list_all_feedback,
    )
)]
pub struct FeedbackApiDoc;

/// Build the feedback router with OpenAPI metadata.
///
/// Generic over router state `S` so this module names no root type: the
/// composition root supplies the concrete state, and each handler reaches its
/// ports through `FromRef<S>`.
pub fn feedback_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<FeedbackNotes>: FromRef<S>,
    Arc<TaskCommands>: FromRef<S>,
    Arc<dyn RepositoryReader>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(FeedbackApiDoc::openapi())
        .routes(routes!(handlers::create_feedback))
        .routes(routes!(handlers::set_feedback_status))
        .routes(routes!(handlers::list_task_feedback))
        .routes(routes!(handlers::list_all_feedback))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
