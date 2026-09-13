//! Task event API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::operations::TaskEvents;
use crate::task::operations::TaskCommands;
use moatless_core::http_auth::UserTokenVerifier;

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
/// Generic over `S` so this module names no root type; each handler reaches its
/// ports via `FromRef<S>`.
pub fn task_event_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<TaskCommands>: FromRef<S>,
    Arc<TaskEvents>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(TaskEventApiDoc::openapi())
        .routes(routes!(handlers::list_task_events))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
