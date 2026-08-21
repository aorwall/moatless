//! Workspace API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::credentials::handlers as credential_handlers;
use super::credentials::service::WorkspaceCredentialService;
use super::dao::WorkspaceDao;
use super::git_sync::GitSyncService;
use super::handlers;
use crate::repository::dao::RepositoryDao;
use moatless_backend_core::identity::UserResolver;
use moatless_identity::extractors::UserTokenVerifier;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "workspaces", description = "Workspace CRUD + composition"),
    ),
    paths(
        handlers::create_workspace,
        handlers::list_workspaces,
        handlers::get_workspace,
        handlers::update_workspace,
        handlers::delete_workspace,
        handlers::override_workspace,
        handlers::reset_git_workspace,
        handlers::add_workspace_repo,
        handlers::remove_workspace_repo,
        handlers::set_workspace_primary_repo,
        credential_handlers::list_workspace_credentials,
        credential_handlers::set_workspace_credential,
        credential_handlers::delete_workspace_credential,
    ),
    components(schemas(
        moatless_api_schemas::workspace::WorkspaceCredentialProvider,
        moatless_api_schemas::workspace::WorkspaceCredentialResponse,
        moatless_api_schemas::workspace::SetWorkspaceCredentialRequest,
    ))
)]
pub struct WorkspaceApiDoc;

/// Build the workspace router with OpenAPI metadata.
///
/// Generic over the router state `S` so this module names no root type: the
/// composition root supplies the concrete state at the call site, and each
/// handler reaches its ports — `Arc<GitSyncService>`, `Arc<dyn RepositoryDao>`,
/// `Arc<dyn WorkspaceDao>`, `Arc<dyn UserResolver>`, and the `UserTokenVerifier`
/// behind `AuthUser` —
/// through `FromRef<S>`.
pub fn workspace_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<GitSyncService>: FromRef<S>,
    Arc<dyn RepositoryDao>: FromRef<S>,
    Arc<dyn WorkspaceDao>: FromRef<S>,
    Arc<WorkspaceCredentialService>: FromRef<S>,
    Arc<dyn UserResolver>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(WorkspaceApiDoc::openapi())
        .routes(routes!(
            handlers::create_workspace,
            handlers::list_workspaces
        ))
        .routes(routes!(
            handlers::get_workspace,
            handlers::update_workspace,
            handlers::delete_workspace
        ))
        .routes(routes!(handlers::override_workspace))
        .routes(routes!(handlers::reset_git_workspace))
        .routes(routes!(handlers::add_workspace_repo))
        .routes(routes!(handlers::remove_workspace_repo))
        .routes(routes!(handlers::set_workspace_primary_repo))
        .routes(routes!(credential_handlers::list_workspace_credentials))
        .routes(routes!(
            credential_handlers::set_workspace_credential,
            credential_handlers::delete_workspace_credential
        ))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
