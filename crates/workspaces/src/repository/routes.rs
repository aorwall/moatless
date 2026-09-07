//! Repository API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use moatless_core::db::DbPool;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::dao::RepositoryDao;
use super::handlers;
use crate::workspace::dao::WorkspaceDao;
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
/// Generic over the router state `S` so this module names no root type: the
/// composition root supplies the concrete state at the call site, and each
/// handler reaches its ports — `Arc<dyn RepositoryDao>`, `Arc<GitSyncService>`,
/// `Arc<dyn GitHostTokenIssuer>`, `Arc<dyn ProviderTokenDao>`, the `DbPool`, and
/// the `UserTokenVerifier` behind `AuthUser` — through `FromRef<S>`. The
/// `sync_config` endpoint lives in `repository_sync` (server band), which is
/// what frees this bound from being upward.
pub fn repository_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn RepositoryDao>: FromRef<S>,
    Arc<dyn WorkspaceDao>: FromRef<S>,
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
