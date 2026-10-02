//! Artifact API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use super::handlers;
use super::operations::Artifacts;
use crate::task::operations::TaskCommands;
use moatless_core::http_auth::UserTokenVerifier;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "artifacts", description = "HTML pages published from a Task"),
    ),
    paths(
        handlers::publish_artifact,
        handlers::list_task_artifacts,
        handlers::open_latest_artifact,
        handlers::serve_artifact_page,
    )
)]
pub struct ArtifactApiDoc;

/// Build the artifact router with OpenAPI metadata, generic over router state
/// `S` like [`crate::feedback::routes::feedback_router`].
pub fn artifact_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<Artifacts>: FromRef<S>,
    Arc<TaskCommands>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(ArtifactApiDoc::openapi())
        .routes(routes!(handlers::publish_artifact).layer(handlers::PUBLISH_BODY_LIMIT))
        .routes(routes!(handlers::list_task_artifacts))
        .routes(routes!(handlers::open_latest_artifact))
        .routes(routes!(handlers::serve_artifact_page))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
