//! Command API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "commands", description = "Command management"),
    ),
    paths(
        handlers::create_command,
        handlers::list_commands,
        handlers::get_command,
        handlers::update_command,
        handlers::delete_command,
        handlers::get_available_commands,
    )
)]
pub struct CommandApiDoc;

/// Build the command router with OpenAPI metadata.
pub fn command_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(CommandApiDoc::openapi())
        .routes(routes!(handlers::create_command, handlers::list_commands))
        .routes(routes!(handlers::get_command, handlers::update_command, handlers::delete_command))
        .routes(routes!(handlers::get_available_commands))
        .with_state(state)
        .split_for_parts();

    (router.into(), api)
}
