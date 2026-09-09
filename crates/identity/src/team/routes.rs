//! Team API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::state::TeamState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "teams", description = "Team management"),
    ),
    paths(
        handlers::list_teams,
        handlers::create_team,
        handlers::get_team,
        handlers::update_team,
        handlers::delete_team,
        handlers::list_members,
        handlers::add_member,
        handlers::remove_member,
    )
)]
pub struct TeamApiDoc;

/// Build the team router with OpenAPI metadata.
pub(crate) fn team_router(state: TeamState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(TeamApiDoc::openapi())
        .routes(routes!(handlers::list_teams, handlers::create_team))
        .routes(routes!(
            handlers::get_team,
            handlers::update_team,
            handlers::delete_team
        ))
        .routes(routes!(handlers::list_members, handlers::add_member))
        .routes(routes!(handlers::remove_member))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
