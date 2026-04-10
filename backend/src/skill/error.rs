//! Skill error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Repository not found: {0}")]
    RepositoryNotFound(String),

    #[error("Authentication required to fetch {git_url}")]
    AuthRequired { git_url: String },

    #[error("Unsupported git provider for {git_url} (only GitHub is supported)")]
    UnsupportedProvider { git_url: String },

    #[error("Skill fetch failed for {git_url}: {message}")]
    Fetch { git_url: String, message: String },

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for SkillError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            SkillError::RepositoryNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            SkillError::AuthRequired { git_url } => {
                tracing::warn!(git_url = %git_url, "Skill fetch blocked by authentication");
                (StatusCode::BAD_GATEWAY, self.to_string())
            }
            SkillError::UnsupportedProvider { git_url } => {
                tracing::warn!(git_url = %git_url, "Skill fetch rejected: unsupported provider");
                (StatusCode::BAD_GATEWAY, self.to_string())
            }
            SkillError::Fetch { git_url, message } => {
                tracing::warn!(git_url = %git_url, error = %message, "Skill fetch failed");
                (StatusCode::BAD_GATEWAY, self.to_string())
            }
            SkillError::Db(e) => {
                tracing::error!(error = %e, "Database error in skill handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            SkillError::Internal(msg) => {
                tracing::error!(error = %msg, "Internal error in skill handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        error_response(status, message)
    }
}
