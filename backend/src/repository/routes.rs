//! Repository API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use moatless_core::db::DbPool;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::access::RepositoryTokenResolver;
use super::dao::RepositoryDao;
use super::handlers;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::extractors::UserTokenVerifier;
use crate::workspace::git_sync::GitSyncService;

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
        handlers::verify_access,
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
/// `Arc<RepositoryTokenResolver>`, `Arc<dyn ProviderTokenDao>`, the `DbPool`, and
/// the `UserTokenVerifier` behind `AuthUser` — through `FromRef<S>`. The
/// `sync_config` endpoint that once forced this
/// module to name `Arc<LoopSyncService>` now lives in `repository_sync` (server
/// band), which is what frees this bound from being upward.
pub fn repository_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn RepositoryDao>: FromRef<S>,
    Arc<GitSyncService>: FromRef<S>,
    Arc<RepositoryTokenResolver>: FromRef<S>,
    Arc<dyn ProviderTokenDao>: FromRef<S>,
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
        .routes(routes!(handlers::verify_access))
        .routes(routes!(handlers::convert_to_template))
        .routes(routes!(handlers::get_branches))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
