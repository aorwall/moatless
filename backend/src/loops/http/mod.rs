//! Loop HTTP surface: the documented `/api/v1/loops` CRUD routes.

pub mod handlers;

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags((name = "loops", description = "Loops — standing definitions of recurring agent work")),
    paths(
        handlers::list_loops,
        handlers::create_loop,
        handlers::get_loop,
        handlers::update_loop,
        handlers::delete_loop,
        handlers::list_loop_recent_tasks,
        handlers::activate_loop,
        handlers::pause_loop,
        handlers::resume_loop,
        handlers::override_loop,
        handlers::reset_git_loop,
    )
)]
pub struct LoopsApiDoc;

/// Build the Loop router.
pub fn loops_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(LoopsApiDoc::openapi())
        .routes(routes!(handlers::list_loops, handlers::create_loop))
        .routes(routes!(handlers::get_loop))
        .routes(routes!(handlers::update_loop, handlers::delete_loop))
        .routes(routes!(handlers::list_loop_recent_tasks))
        .routes(routes!(handlers::activate_loop))
        .routes(routes!(handlers::pause_loop))
        .routes(routes!(handlers::resume_loop))
        .routes(routes!(handlers::override_loop))
        .routes(routes!(handlers::reset_git_loop))
        .with_state(state.clone())
        .split_for_parts();

    (router, api)
}
