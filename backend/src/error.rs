//! Shared error response helper for all API modules.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

/// Standard JSON error body. All API errors MUST use this shape.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
}

/// Build a standard JSON error response: `{"error": "message"}`.
///
/// Every module's `IntoResponse` impl MUST use this function to guarantee
/// a consistent response shape across all endpoints.
pub fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
        .into_response()
}
