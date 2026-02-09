//! File storage API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "file-storage", description = "File upload and download via S3"),
    ),
    paths(
        handlers::upload_file,
        handlers::download_file,
    )
)]
pub struct FileStorageApiDoc;

/// Build the file storage router with OpenAPI metadata.
pub fn file_storage_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(FileStorageApiDoc::openapi())
        .routes(routes!(handlers::upload_file))
        .routes(routes!(handlers::download_file))
        .with_state(state)
        .split_for_parts();

    (router.into(), api)
}
