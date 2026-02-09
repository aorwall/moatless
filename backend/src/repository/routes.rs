//! Repository API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "repositories", description = "Repository CRUD operations"),
    ),
    paths(
        handlers::create_repository,
        handlers::list_repositories,
        handlers::get_repository,
        handlers::update_repository,
        handlers::delete_repository,
        handlers::convert_to_template,
        handlers::get_branches,
    )
)]
pub struct RepositoryApiDoc;

/// Build the repository router with OpenAPI metadata.
pub fn repository_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(RepositoryApiDoc::openapi())
        .routes(routes!(handlers::create_repository, handlers::list_repositories))
        .routes(routes!(handlers::get_repository, handlers::update_repository, handlers::delete_repository))
        .routes(routes!(handlers::convert_to_template))
        .routes(routes!(handlers::get_branches))
        .with_state(state)
        .split_for_parts();

    (router.into(), api)
}
