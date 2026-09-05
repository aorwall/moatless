//! Message API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::message::dao::MessageDao;
use crate::message::send::MessageSendService;
use crate::task::dao::TaskDao;
use moatless_core::file_storage::dao::FileDao;
use moatless_core::http_auth::UserTokenVerifier;
use moatless_ports::sandbox::SandboxWaker;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "messages", description = "Message sending, retrieval, and management"),
    ),
    paths(
        handlers::send_message,
        handlers::get_messages,
        handlers::get_pending_messages,
        handlers::get_tool_call_detail,
        handlers::get_subagent_transcript,
    )
)]
pub struct MessageApiDoc;

/// Build the message router with OpenAPI metadata.
///
/// Generic over the router state `S`: each handler extracts its ports through
/// `FromRef<S>`, so this router names those ports (tasks-band DAOs plus the
/// message-send service and sandbox waker) rather than the whole `AppState`.
/// The composition root passes `AppState` as `S`.
pub fn message_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
    Arc<dyn TaskDao>: FromRef<S>,
    Arc<dyn MessageDao>: FromRef<S>,
    Arc<dyn FileDao>: FromRef<S>,
    Arc<MessageSendService>: FromRef<S>,
    Arc<dyn SandboxWaker>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(MessageApiDoc::openapi())
        .routes(routes!(handlers::send_message, handlers::get_messages))
        .routes(routes!(handlers::get_pending_messages))
        .routes(routes!(handlers::get_tool_call_detail))
        .routes(routes!(handlers::get_subagent_transcript))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
