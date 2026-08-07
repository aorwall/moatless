//! Skill error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::error_response;

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

    /// Resolving the repository-scoped git token a workspace skill fetch needs
    /// failed. A plain "no token configured" is not an error and never reaches
    /// here.
    #[error("Repository access token could not be resolved")]
    RepositoryToken(#[source] crate::repository::error::RepositoryError),

    /// Resolving the caller's own provider token failed. Retains the auth
    /// failure so the log says *which* credential path broke.
    #[error("User provider token could not be resolved")]
    UserToken(#[source] moatless_identity::auth::error::AuthError),

    /// An invariant this module synthesizes, with no underlying failure to
    /// carry — kept distinct from the wrapping variants above.
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
            SkillError::Db(_) | SkillError::RepositoryToken(_) | SkillError::UserToken(_) => {
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
    use moatless_identity::auth::error::AuthError;

    #[test]
    fn token_resolution_keeps_the_underlying_failure_as_source() {
        let err = SkillError::UserToken(AuthError::Config(
            "GITHUB_APP_PRIVATE_KEY is not a valid PEM".to_string(),
        ));
        let source = std::error::Error::source(&err).expect("auth error is the source");
        assert_eq!(
            source.to_string(),
            "Configuration error: GITHUB_APP_PRIVATE_KEY is not a valid PEM"
        );

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
        let err = SkillError::UserToken(AuthError::OAuth(
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
