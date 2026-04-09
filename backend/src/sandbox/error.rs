//! Unified error type for sandbox operations.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_response;

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

    #[error("Docker API error: {0}")]
    DockerError(String),

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
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::JwtError(ref msg) => {
                tracing::error!(error = %msg, "Sandbox JWT error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::PodBuildError(ref msg) => {
                tracing::error!(error = %msg, "Sandbox pod build error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::KubeError(ref e) => {
                tracing::error!(error = %e, "Kubernetes API error in sandbox handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::DockerError(ref msg) => {
                tracing::error!(error = %msg, "Docker API error in sandbox handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal sandbox error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };
        error_response(status, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            SandboxError::ConfigMissing("TOKEN_SECRET".into()).to_string(),
            "Configuration missing: TOKEN_SECRET"
        );
        assert_eq!(
            SandboxError::JwtError("expired".into()).to_string(),
            "JWT generation failed: expired"
        );
        assert_eq!(
            SandboxError::PodBuildError("oom".into()).to_string(),
            "Pod build failed: oom"
        );
        assert_eq!(
            SandboxError::NotReady("starting".into()).to_string(),
            "Sandbox not ready: starting"
        );
        assert_eq!(
            SandboxError::Internal("crash".into()).to_string(),
            "Internal error: crash"
        );
    }

    #[tokio::test]
    async fn not_ready_returns_503() {
        let resp = SandboxError::NotReady("still starting".into()).into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn config_missing_returns_500() {
        let resp = SandboxError::ConfigMissing("secret".into()).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn internal_returns_500() {
        let resp = SandboxError::Internal("boom".into()).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn internal_errors_do_not_leak_details() {
        let resp = SandboxError::ConfigMissing("TOKEN_SECRET".into()).into_response();
        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Should NOT expose the actual config key
        let error_msg = json["error"].as_str().unwrap();
        assert!(!error_msg.contains("TOKEN_SECRET"));
        assert_eq!(error_msg, "Internal server error");
    }
}
