//! Team API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::dao::TeamDao;
use super::handlers;
use crate::extractors::UserTokenVerifier;

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
///
/// Generic over the router state `S` so this module names no root type: the
/// composition root supplies the concrete state at the call site, and each
/// handler reaches its ports — `Arc<dyn TeamDao>` and the `UserTokenVerifier`
/// behind `AuthUser` — through `FromRef<S>`.
pub fn team_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn TeamDao>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
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
