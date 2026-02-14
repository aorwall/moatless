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
            Self::ConfigMissing(ref msg) => {
                tracing::error!(error = %msg, "Sandbox config missing");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            Self::JwtError(ref msg) => {
                tracing::error!(error = %msg, "Sandbox JWT error");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            Self::PodBuildError(ref msg) => {
                tracing::error!(error = %msg, "Sandbox pod build error");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            Self::KubeError(ref e) => {
                tracing::error!(error = %e, "Kubernetes API error in sandbox handler");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            Self::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal sandbox error");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
        };
        (status, Json(serde_json::json!({"error": message}))).into_response()
    }
}
