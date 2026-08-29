//! Skill error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Repository not found: {0}")]
    RepositoryNotFound(String),

    #[error("Workspace not found: {0}")]
    WorkspaceNotFound(String),

    #[error("Authentication required to fetch {git_url}")]
    AuthRequired { git_url: String },

    #[error("Unsupported git provider for {git_url} (only GitHub is supported)")]
    UnsupportedProvider { git_url: String },

    #[error("Skill fetch failed for {git_url}: {message}")]
    Fetch { git_url: String, message: String },

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    /// Resolving the git-host token a workspace skill fetch needs failed. A
    /// plain "not connected" is not an error and never reaches here.
    #[error("Repository access token could not be resolved")]
    RepositoryToken(#[source] crate::repository::error::RepositoryError),

    /// An invariant this module synthesizes, with no underlying failure to
    /// carry — kept distinct from the wrapping variants above.
    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for SkillError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            SkillError::RepositoryNotFound(_) | SkillError::WorkspaceNotFound(_) => {
                (StatusCode::NOT_FOUND, self.to_string())
            }
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
            SkillError::Db(_) | SkillError::RepositoryToken(_) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    "Internal error in skill handler"
                );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_resolution_keeps_the_underlying_failure_as_source() {
        let err = SkillError::RepositoryToken(crate::repository::error::RepositoryError::Db(
            sqlx::Error::Protocol("SELECT access_token FROM provider_tokens".to_string()),
        ));
        let source = std::error::Error::source(&err).expect("repository error is the source");
        assert!(source.to_string().contains("SELECT access_token"));
    }

    #[test]
    fn synthesized_internal_messages_have_no_source() {
        let err = SkillError::Internal("skill cache not wired".to_string());
        assert!(std::error::Error::source(&err).is_none());
    }

    #[tokio::test]
    async fn token_resolution_responds_500_without_credential_detail() {
        let err =
            SkillError::RepositoryToken(crate::repository::error::RepositoryError::ProviderError(
                "installation token ghs_secretvalue rejected".to_string(),
            ));
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "error": "Internal server error" })
        );
    }
}
