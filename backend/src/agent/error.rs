//! Agent module error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AgentError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::Internal(msg) => {
                tracing::error!(error = %msg, "Internal error in agent handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };
        error_response(status, message)
    }
}
