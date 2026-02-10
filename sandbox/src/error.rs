//! Sandbox error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

/// Unified error type for the sandbox service.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum SandboxError {
    #[error("Agent is already running")]
    AgentAlreadyRunning,

    #[error("Agent is not running")]
    AgentNotRunning,

    #[error("No pending interactive prompt")]
    NoPendingPrompt,

    #[error("Server not found: {0}")]
    ServerNotFound(String),

    #[error("SDK error: {0}")]
    Sdk(#[from] claude_agent_sdk::SdkError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("gRPC error: {0}")]
    Grpc(#[from] tonic::Status),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for SandboxError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::AgentAlreadyRunning => (StatusCode::CONFLICT, self.to_string()),
            Self::AgentNotRunning => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::NoPendingPrompt => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::ServerNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
