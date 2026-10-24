//! Workspace API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::credentials::handlers as credential_handlers;
use super::credentials::service::WorkspaceCredentialService;
use super::handlers;
use super::operations::Workspaces;
use moatless_core::http_auth::UserTokenVerifier;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "workspaces", description = "Workspaces synced from git, and their credentials"),
    ),
    paths(
        handlers::list_workspaces,
        handlers::get_workspace,
        handlers::delete_workspace,
        credential_handlers::list_workspace_credentials,
        credential_handlers::set_workspace_credential,
        credential_handlers::delete_workspace_credential,
    ),
    components(schemas(
        moatless_workspace_types::schemas::workspace::WorkspaceCredentialProvider,
        moatless_workspace_types::schemas::workspace::WorkspaceCredentialResponse,
        moatless_workspace_types::schemas::workspace::SetWorkspaceCredentialRequest,
    ))
)]
pub struct WorkspaceApiDoc;

/// Build the workspace router with OpenAPI metadata.
///
/// Generic over `S` so this module names no root type; handlers reach their
/// ports (`Arc<Workspaces>`, `Arc<WorkspaceCredentialService>`,
/// `UserTokenVerifier`) through `FromRef<S>`.
pub fn workspace_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<Workspaces>: FromRef<S>,
    Arc<WorkspaceCredentialService>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(WorkspaceApiDoc::openapi())
        .routes(routes!(handlers::list_workspaces))
        .routes(routes!(handlers::get_workspace, handlers::delete_workspace))
        .routes(routes!(credential_handlers::list_workspace_credentials))
        .routes(routes!(
            credential_handlers::set_workspace_credential,
            credential_handlers::delete_workspace_credential
        ))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
