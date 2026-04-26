//! Message API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "messages", description = "Message sending, retrieval, and management"),
    ),
    paths(
        handlers::send_message,
        handlers::get_messages,
        handlers::get_pending_messages,
        handlers::delete_queued_message,
        handlers::get_tool_call_detail,
    )
)]
pub struct MessageApiDoc;

/// Build the message router with OpenAPI metadata.
pub fn message_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(MessageApiDoc::openapi())
        .routes(routes!(handlers::send_message, handlers::get_messages))
        .routes(routes!(handlers::get_pending_messages))
        .routes(routes!(handlers::delete_queued_message))
        .routes(routes!(handlers::get_tool_call_detail))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
