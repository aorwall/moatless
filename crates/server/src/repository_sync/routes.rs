//! Repository config-sync API route with OpenAPI documentation.
//!
//! The operation keeps `tag = "repositories"`: the OpenAPI tag is the external
//! contract that drives the frontend's tags-split client codegen, so it stays
//! grouped under `repositories` even though the handler lives here. This `ApiDoc`
//! declares no tag of its own; `repository::routes::RepositoryApiDoc` owns the
//! `repositories` tag description.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    paths(
        handlers::sync_config,
    )
)]
pub struct RepositorySyncApiDoc;

/// Build the repository config-sync router with OpenAPI metadata.
pub fn repository_sync_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(RepositorySyncApiDoc::openapi())
        .routes(routes!(handlers::sync_config))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
