//! Team API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

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
        handlers::get_settings,
        handlers::update_settings,
        handlers::list_credentials,
        handlers::upsert_credential,
        handlers::delete_credential,
    )
)]
pub struct TeamApiDoc;

/// Build the team router with OpenAPI metadata.
pub fn team_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(TeamApiDoc::openapi())
        .routes(routes!(handlers::list_teams, handlers::create_team))
        .routes(routes!(handlers::get_team, handlers::update_team, handlers::delete_team))
        .routes(routes!(handlers::list_members, handlers::add_member))
        .routes(routes!(handlers::remove_member))
        .routes(routes!(handlers::get_settings, handlers::update_settings))
        .routes(routes!(handlers::list_credentials))
        .routes(routes!(handlers::upsert_credential, handlers::delete_credential))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
