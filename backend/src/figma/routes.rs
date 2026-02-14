//! Figma API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "figma", description = "Figma design import and clipboard parsing"),
    ),
    paths(
        handlers::import_clipboard,
        handlers::import_node,
        handlers::parse_clipboard,
        handlers::get_import,
        handlers::get_import_image,
    )
)]
pub struct FigmaApiDoc;

/// Build the figma router with OpenAPI metadata.
pub fn figma_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(FigmaApiDoc::openapi())
        .routes(routes!(handlers::import_clipboard))
        .routes(routes!(handlers::import_node))
        .routes(routes!(handlers::parse_clipboard))
        .routes(routes!(handlers::get_import))
        .routes(routes!(handlers::get_import_image))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
