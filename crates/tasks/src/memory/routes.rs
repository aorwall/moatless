//! Memory API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::handlers;
use super::operations::Memories;
use crate::task::operations::TaskCommands;
use moatless_core::http_auth::UserTokenVerifier;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "memory", description = "What agents learn, shared across Tasks by scope"),
    ),
    paths(
        handlers::list_memory,
        handlers::get_memory,
        handlers::put_memory,
        handlers::delete_memory,
        handlers::search_memory,
        handlers::memory_log,
        handlers::append_memory,
        handlers::verify_memory,
        handlers::claim_memory,
        handlers::release_memory,
    )
)]
pub struct MemoryApiDoc;

/// Build the memory router with OpenAPI metadata, generic over router state
/// `S` like [`crate::artifact::routes::artifact_router`].
pub fn memory_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<Memories>: FromRef<S>,
    Arc<TaskCommands>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(MemoryApiDoc::openapi())
        .routes(routes!(handlers::list_memory))
        .routes(routes!(
            handlers::get_memory,
            handlers::put_memory,
            handlers::delete_memory
        ))
        .routes(routes!(handlers::search_memory))
        .routes(routes!(handlers::memory_log))
        .routes(routes!(handlers::append_memory))
        .routes(routes!(handlers::verify_memory))
        .routes(routes!(handlers::claim_memory))
        .routes(routes!(handlers::release_memory))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
