//! Tag API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::operations::Tags;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "tags", description = "Tag listing"),
    ),
    paths(
        handlers::list_tags,
    )
)]
pub struct TagApiDoc;

/// Build the tag router with OpenAPI metadata.
///
/// Generic over `S` so this tasks-band module names no root type — the one
/// handler reaches its port through `Arc<Tags>: FromRef<S>`, and `with_state`
/// erases `S` before `split_for_parts` so the bound never escapes this
/// function.
pub fn tag_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<Tags>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(TagApiDoc::openapi())
        .routes(routes!(handlers::list_tags))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
