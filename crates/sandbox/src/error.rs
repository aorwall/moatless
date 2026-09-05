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
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            SandboxError::AgentAlreadyRunning.to_string(),
            "Agent is already running"
        );
        assert_eq!(
            SandboxError::AgentNotRunning.to_string(),
            "Agent is not running"
        );
        assert_eq!(
            SandboxError::NoPendingPrompt.to_string(),
            "No pending interactive prompt"
        );
        assert_eq!(
            SandboxError::Internal("custom".to_string()).to_string(),
            "Internal error: custom"
        );
    }

    #[tokio::test]
    async fn error_into_response_status_codes() {
        let resp = SandboxError::AgentAlreadyRunning.into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);

        let resp = SandboxError::AgentNotRunning.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp = SandboxError::NoPendingPrompt.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp = SandboxError::Internal("boom".into()).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
