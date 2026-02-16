//! Tag API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

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
pub fn tag_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(TagApiDoc::openapi())
        .routes(routes!(handlers::list_tags))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
