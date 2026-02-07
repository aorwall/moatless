//! Unified error type for sandbox operations.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

/// Errors that can occur during sandbox lifecycle operations.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("Configuration missing: {0}")]
    ConfigMissing(String),

    #[error("JWT generation failed: {0}")]
    JwtError(String),

    #[error("Kubernetes API error: {0}")]
    KubeError(#[from] kube::Error),

    #[error("Pod build failed: {0}")]
    PodBuildError(String),

    #[error("Sandbox not ready: {0}")]
    NotReady(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for SandboxError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotReady(_) => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            Self::ConfigMissing(_) | Self::JwtError(_) | Self::PodBuildError(_) | Self::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            Self::KubeError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        (status, Json(serde_json::json!({"error": message}))).into_response()
    }
}
