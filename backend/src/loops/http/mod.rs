//! Loop HTTP surface: the documented `/api/v1/loops` CRUD routes plus the
//! deprecated, undocumented `/api/v1/triggers` alias that serves the same
//! handlers for existing clients.

pub mod handlers;

use axum::routing::get;
use axum::Router;
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

/// Build the Loop router: documented `/api/v1/loops` routes plus the
/// undocumented `/api/v1/triggers` alias against the same handlers.
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

    (router.merge(legacy_trigger_aliases(state)), api)
}

/// Deprecated `/api/v1/triggers` alias. Registered against the same handlers as
/// `/api/v1/loops` but kept out of the OpenAPI document (exact precedent:
/// `legacy_webhook_aliases()` in `adapters/routes.rs`).
fn legacy_trigger_aliases(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/v1/triggers",
            get(handlers::list_loops).post(handlers::create_loop),
        )
        .route(
            "/api/v1/triggers/{id}",
            get(handlers::get_loop)
                .put(handlers::update_loop)
                .delete(handlers::delete_loop),
        )
        .route(
            "/api/v1/triggers/{id}/tasks",
            get(handlers::list_loop_recent_tasks),
        )
        .route(
            "/api/v1/triggers/{id}/activate",
            axum::routing::post(handlers::activate_loop),
        )
        .route(
            "/api/v1/triggers/{id}/pause",
            axum::routing::post(handlers::pause_loop),
        )
        .route(
            "/api/v1/triggers/{id}/resume",
            axum::routing::post(handlers::resume_loop),
        )
        .route(
            "/api/v1/triggers/{id}/override",
            axum::routing::post(handlers::override_loop),
        )
        .route(
            "/api/v1/triggers/{id}/reset-git",
            axum::routing::post(handlers::reset_git_loop),
        )
        .with_state(state)
}
