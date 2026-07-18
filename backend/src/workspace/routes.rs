//! Workspace API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

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
        handlers::add_workspace_repo,
        handlers::remove_workspace_repo,
        handlers::set_workspace_primary_repo,
    )
)]
pub struct WorkspaceApiDoc;

/// Build the workspace router with OpenAPI metadata.
pub fn workspace_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
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
        .routes(routes!(handlers::add_workspace_repo))
        .routes(routes!(handlers::remove_workspace_repo))
        .routes(routes!(handlers::set_workspace_primary_repo))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
