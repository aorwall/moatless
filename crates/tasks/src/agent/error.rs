//! Agent module error types.

use axum::response::{IntoResponse, Response};

use moatless_core::error::{error_response, internal_error};

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
                internal_error()
            }
        };
        error_response(status, message)
    }
}
