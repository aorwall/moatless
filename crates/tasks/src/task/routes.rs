//! Task API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::credential_sync::CredentialSync;
use crate::message::operations::Messages;
use crate::message::send::MessageSendService;
use crate::tag::operations::Tags;
use crate::task::bindings::TaskBindings;
use crate::task::closer::TaskCloser;
use crate::task::creation::TaskCreationService;
use crate::task::fork::TaskForkService;
use crate::task::operations::TaskCommands;
use crate::task::presenter::TaskPresenter;
use crate::task_turn::operations::TaskTurns;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_core::http_auth::UserTokenVerifier;
use moatless_ports::identity::UserResolver;
use moatless_ports::sandbox::PreflightPort;
use moatless_ports::sandbox::SandboxLifecycle;
use moatless_ports::workspace::{RepositoryReader, WorkspaceResolver};
use moatless_workspaces::workspace::git_sync::GitSyncService;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "tasks", description = "Task CRUD operations and lifecycle"),
    ),
    paths(
        handlers::create_task,
        handlers::fork_task,
        handlers::list_tasks,
        handlers::get_task,
        handlers::get_task_statistics,
        handlers::update_task_metadata,
        handlers::delete_task,
        handlers::mark_task_read,
        handlers::follow_task,
        handlers::unfollow_task,
        handlers::close_task,
        handlers::reopen_task,
        handlers::start_task,
        handlers::stop_task,
        handlers::set_task_tags,
        handlers::update_visibility,
        handlers::update_ttl,
    )
)]
pub struct TaskApiDoc;

/// Build the task router with OpenAPI metadata. Generic over state `S` so this
/// module names no root type — the composition root supplies it, and handlers
/// reach their ports through `FromRef<S>`.
pub fn task_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<TaskCommands>: FromRef<S>,
    Arc<dyn TaskBindings>: FromRef<S>,
    Arc<TaskTurns>: FromRef<S>,
    Arc<Messages>: FromRef<S>,
    Arc<Tags>: FromRef<S>,
    Arc<dyn RepositoryReader>: FromRef<S>,
    Arc<dyn WorkspaceResolver>: FromRef<S>,
    Arc<dyn SandboxLifecycle>: FromRef<S>,
    Arc<dyn PreflightPort>: FromRef<S>,
    Arc<dyn UserResolver>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
    Arc<TaskPresenter>: FromRef<S>,
    Arc<TaskCloser>: FromRef<S>,
    Arc<TaskCreationService>: FromRef<S>,
    Arc<TaskForkService>: FromRef<S>,
    Arc<MessageSendService>: FromRef<S>,
    Arc<CredentialSync>: FromRef<S>,
    Arc<GitSyncService>: FromRef<S>,
    Arc<Config>: FromRef<S>,
    Option<Arc<dyn EventPublisher>>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(TaskApiDoc::openapi())
        .routes(routes!(handlers::create_task, handlers::list_tasks))
        .routes(routes!(
            handlers::get_task,
            handlers::update_task_metadata,
            handlers::delete_task
        ))
        .routes(routes!(handlers::fork_task))
        .routes(routes!(handlers::get_task_statistics))
        .routes(routes!(handlers::close_task))
        .routes(routes!(handlers::reopen_task))
        .routes(routes!(handlers::mark_task_read))
        .routes(routes!(handlers::follow_task, handlers::unfollow_task))
        .routes(routes!(handlers::start_task))
        .routes(routes!(handlers::stop_task))
        .routes(routes!(handlers::set_task_tags))
        .routes(routes!(handlers::update_visibility))
        .routes(routes!(handlers::update_ttl))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
