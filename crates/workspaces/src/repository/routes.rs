//! Repository API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use moatless_core::db::DbPool;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::operations::Repositories;
use crate::workspace::Workspaces;
use crate::workspace::git_sync::GitSyncService;
use moatless_core::http_auth::UserTokenVerifier;

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
///
/// Generic over `S` so this module names no root type; handlers reach their
/// ports through `FromRef<S>`. The `sync_config` endpoint lives in
/// `repository_sync` (server band) instead, keeping this bound from pointing
/// upward.
pub fn repository_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<Repositories>: FromRef<S>,
    Arc<Workspaces>: FromRef<S>,
    Arc<GitSyncService>: FromRef<S>,
    Arc<super::branches::RepositoryBranchService>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
    DbPool: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(RepositoryApiDoc::openapi())
        .routes(routes!(
            handlers::create_repository,
            handlers::list_repositories
        ))
        .routes(routes!(
            handlers::get_repository,
            handlers::update_repository,
            handlers::delete_repository
        ))
        .routes(routes!(handlers::convert_to_template))
        .routes(routes!(handlers::get_branches))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
