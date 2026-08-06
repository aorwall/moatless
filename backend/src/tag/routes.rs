//! Tag API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::dao::TagDao;
use super::handlers;

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
/// Generic over the router state `S` so this core-band module names no root
/// type: the composition root supplies the concrete state at the call site, and
/// the one handler reaches its one port through `Arc<dyn TagDao>: FromRef<S>`.
/// `with_state` erases `S` before `split_for_parts`, so the bound stays local to
/// this function and never appears in the returned router.
pub fn tag_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn TagDao>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(TagApiDoc::openapi())
        .routes(routes!(handlers::list_tags))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
