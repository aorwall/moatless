//! Channel error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use moatless_api_schemas::channel::{ChannelTargetDescriptor, MultipleTargetsErrorBody};

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("Channel config not found: {0}")]
    ConfigNotFound(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Provider not configured: {0}")]
    ProviderNotConfigured(String),

    #[error("Signature verification failed")]
    SignatureInvalid,

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Provider API error: {0}")]
    ProviderApi(String),

    #[error("Access denied: {0}")]
    AccessDenied(String),

    /// Returned by the no-flag send endpoint when a task has more than one
    /// channel destination — the caller must pick one with `--thread`,
    /// `--subscription`, or `--pr`.
    #[error("Task has multiple channel destinations; pick one explicitly")]
    MultipleTargets {
        targets: Vec<ChannelTargetDescriptor>,
    },

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ChannelError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ChannelError::ConfigNotFound(_) | ChannelError::NotFound(_) => {
                (StatusCode::NOT_FOUND, self.to_string())
            }
            ChannelError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ChannelError::ProviderNotConfigured(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ChannelError::SignatureInvalid => (StatusCode::UNAUTHORIZED, self.to_string()),
            ChannelError::Db(e) => {
                tracing::error!(error = %e, "Database error in channel handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            ChannelError::AccessDenied(_) => (StatusCode::FORBIDDEN, self.to_string()),
            ChannelError::ProviderApi(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            ChannelError::MultipleTargets { targets } => {
                let body = MultipleTargetsErrorBody {
                    error: self.to_string(),
                    targets: targets.clone(),
                    hint:
                        "Use --thread <id>, --subscription <id>, or --pr owner/repo#N to pick one."
                            .to_string(),
                };
                return (StatusCode::BAD_REQUEST, Json(body)).into_response();
            }
            ChannelError::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal error in channel handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        error_response(status, message)
    }
}
