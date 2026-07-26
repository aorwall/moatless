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

/// Render an error together with its full `source` chain as one line.
///
/// Module errors keep their causal failure as a `#[source]`, but `Display`
/// deliberately shows only the outermost message. The transport boundary is
/// the one place the whole chain is wanted: `IntoResponse` logs
/// `causal_chain(&self)` once and returns a redacted body, so nothing that was
/// wrapped is lost and nothing is logged twice on the way up.
pub fn causal_chain(error: &dyn std::error::Error) -> String {
    let mut rendered = error.to_string();
    let mut next = error.source();
    while let Some(cause) = next {
        rendered.push_str(": ");
        rendered.push_str(&cause.to_string());
        next = cause.source();
    }
    rendered
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

#[cfg(test)]
mod tests {
    use super::causal_chain;

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Middle);

    #[derive(Debug, thiserror::Error)]
    #[error("middle")]
    struct Middle(#[source] std::io::Error);

    #[test]
    fn causal_chain_renders_every_source() {
        let err = Outer(Middle(std::io::Error::other("inner")));
        assert_eq!(causal_chain(&err), "outer: middle: inner");
    }

    #[test]
    fn causal_chain_of_a_sourceless_error_is_its_display() {
        assert_eq!(causal_chain(&std::io::Error::other("alone")), "alone");
    }
}
