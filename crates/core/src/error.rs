//! Shared error response helper for all API modules.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

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
/// a consistent response shape across all endpoints. The body is built inline
/// so this crate names no schema crate; it must stay identical to
/// `moatless_api_schemas::error::ErrorBody`, which is what the OpenAPI spec
/// documents for every one of these responses.
pub fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::{causal_chain, error_response};
    use axum::http::StatusCode;

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

    /// The rendered body is the wire contract every endpoint returns on an
    /// error, so it is asserted byte-for-byte rather than through a type.
    #[tokio::test]
    async fn error_response_renders_exactly_one_error_field() {
        let response = error_response(StatusCode::NOT_FOUND, "task not found");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is in memory");
        assert_eq!(&body[..], br#"{"error":"task not found"}"#);
    }
}
