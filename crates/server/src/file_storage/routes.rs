//! File storage API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use moatless_core::file_storage::Files;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use moatless_core::http_auth::UserTokenVerifier;

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
///
/// Generic over the router state `S` so this module names no root type: the
/// composition root supplies the concrete state at the call site, and each
/// handler reaches its ports — `Arc<Files>` and the `UserTokenVerifier` behind
/// `AuthUser` — through `FromRef<S>`.
pub fn file_storage_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<Files>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(FileStorageApiDoc::openapi())
        .routes(routes!(handlers::upload_file))
        .routes(routes!(handlers::download_file))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
