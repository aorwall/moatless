//! Loop HTTP surface: the documented `/api/v1/loops` CRUD routes.

pub mod handlers;

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::adapters::dao::AdapterConnectionDao;
use crate::auth::users::UserDao;
use crate::task::binding_dao::TaskBindingDao;
use crate::extractors::UserTokenVerifier;
use crate::loops::dao::LoopDao;
use crate::loops::sync::LoopSyncService;
use crate::repository::dao::RepositoryDao;

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
///
/// Generic over the router state `S` so this module names no root type: the
/// composition root supplies the concrete state at the call site, and each
/// registered handler reaches its ports — the `loops`, `users`,
/// `task_bindings`, `repositories`, `adapter_connections` DAOs, the
/// `LoopSyncService`, and the `UserTokenVerifier` behind `AuthUser` — through
/// `FromRef<S>`.
pub fn loops_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<LoopDao>: FromRef<S>,
    Arc<dyn UserDao>: FromRef<S>,
    Arc<dyn TaskBindingDao>: FromRef<S>,
    Arc<dyn RepositoryDao>: FromRef<S>,
    Arc<LoopSyncService>: FromRef<S>,
    Arc<dyn AdapterConnectionDao>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
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
        .with_state(state)
        .split_for_parts();

    (router, api)
}
