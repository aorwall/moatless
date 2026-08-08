//! Task event API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::dao::TaskEventDao;
use super::handlers;
use moatless_identity::extractors::UserTokenVerifier;
use crate::task::dao::TaskDao;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "task-events", description = "Task event history"),
    ),
    paths(
        handlers::list_task_events,
    )
)]
pub struct TaskEventApiDoc;

/// Build the task event router with OpenAPI metadata.
///
/// Generic over the router state `S` so this module names no root type: the
/// composition root supplies the concrete state at the call site, and each
/// handler reaches its ports — `Arc<dyn TaskDao>`, `Arc<dyn TaskEventDao>`, and
/// the `UserTokenVerifier` behind `AuthUser` — through `FromRef<S>`.
pub fn task_event_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn TaskDao>: FromRef<S>,
    Arc<dyn TaskEventDao>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(TaskEventApiDoc::openapi())
        .routes(routes!(handlers::list_task_events))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
