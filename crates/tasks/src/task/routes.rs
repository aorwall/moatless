//! Task API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::credential_sync::CredentialSync;
use moatless_identity::extractors::UserTokenVerifier;
use crate::message::dao::MessageDao;
use crate::message::send::MessageSendService;
use moatless_workspaces::repository::dao::RepositoryDao;
use crate::task::binding_dao::TaskBindingDao;
use crate::task::closer::TaskCloser;
use crate::task::creation::TaskCreationService;
use moatless_backend_core::sandbox::PreflightPort;
use crate::task::dao::TaskDao;
use crate::task::presenter::TaskPresenter;
use crate::task::relations::TaskRelationDao;
use moatless_backend_core::sandbox::SandboxLifecycle;
use crate::task_turn::dao::TaskTurnDao;
use moatless_workspaces::workspace::dao::WorkspaceDao;
use moatless_workspaces::workspace::git_sync::GitSyncService;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use crate::tag::dao::TagDao;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "tasks", description = "Task CRUD operations and lifecycle"),
    ),
    paths(
        handlers::create_task,
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

/// Build the task router with OpenAPI metadata.
///
/// Generic over the router state `S` so this module names no root type: the
/// composition root supplies the concrete state at the call site, and each
/// registered handler reaches its ports — the task/message/creation services,
/// the task/binding/relation/turn/message DAOs, the repository/workspace/git
/// and tag ports, the sandbox lifecycle and preflight ports, the credential
/// sync and config, and the `UserTokenVerifier` behind `AuthUser` — through
/// `FromRef<S>`.
pub fn task_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn TaskDao>: FromRef<S>,
    Arc<dyn TaskBindingDao>: FromRef<S>,
    Arc<dyn TaskRelationDao>: FromRef<S>,
    Arc<dyn TaskTurnDao>: FromRef<S>,
    Arc<dyn MessageDao>: FromRef<S>,
    Arc<dyn TagDao>: FromRef<S>,
    Arc<dyn RepositoryDao>: FromRef<S>,
    Arc<dyn WorkspaceDao>: FromRef<S>,
    Arc<dyn SandboxLifecycle>: FromRef<S>,
    Arc<dyn PreflightPort>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
    Arc<TaskPresenter>: FromRef<S>,
    Arc<TaskCloser>: FromRef<S>,
    Arc<TaskCreationService>: FromRef<S>,
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
