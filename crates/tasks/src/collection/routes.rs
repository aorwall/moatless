//! Record collection API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::handlers;
use super::operations::Records;
use crate::task::operations::TaskCommands;
use moatless_core::http_auth::UserTokenVerifier;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "collections", description = "Records shared across Tasks by scope"),
    ),
    paths(
        handlers::list_collections,
        handlers::list_collection_records,
        handlers::write_collection_record,
    )
)]
pub struct CollectionApiDoc;

/// Build the collection router with OpenAPI metadata, generic over router
/// state `S` like [`crate::memory::routes::memory_router`].
pub fn collection_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<Records>: FromRef<S>,
    Arc<TaskCommands>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    OpenApiRouter::with_openapi(CollectionApiDoc::openapi())
        .routes(routes!(handlers::list_collections))
        .routes(routes!(
            handlers::list_collection_records,
            handlers::write_collection_record
        ))
        .with_state(state)
        .split_for_parts()
}
