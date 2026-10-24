//! Artifact API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use super::operations::Artifacts;
use super::{handlers, record_handlers};
use crate::message::send::MessageSendService;
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
        record_handlers::write_record,
        record_handlers::list_records,
        record_handlers::write_page_record,
        record_handlers::list_page_records,
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
    Arc<MessageSendService>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(ArtifactApiDoc::openapi())
        .routes(routes!(handlers::publish_artifact).layer(handlers::PUBLISH_BODY_LIMIT))
        .routes(routes!(handlers::list_task_artifacts))
        .routes(routes!(handlers::open_latest_artifact))
        .routes(routes!(handlers::serve_artifact_page))
        .routes(routes!(
            record_handlers::write_record,
            record_handlers::list_records
        ))
        .routes(routes!(
            record_handlers::write_page_record,
            record_handlers::list_page_records
        ))
        .with_state(state.clone())
        .split_for_parts();
    // No OpenAPI path for the socket: utoipa has no WebSocket operation, and
    // its frames are described in `record_socket`.
    let socket = axum::Router::new()
        .route(
            super::record_socket::PAGE_SOCKET_ROUTE,
            axum::routing::get(super::record_socket::page_socket),
        )
        .with_state(state);
    let router = router
        .route(
            super::page_token::PAGE_RECORDS_ROUTE,
            axum::routing::options(record_handlers::page_records_preflight),
        )
        .merge(socket);

    (router, api)
}
