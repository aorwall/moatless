//! Auth error types with axum IntoResponse.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

use moatless_core::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Authentication required")]
    Unauthorized,

    #[error("Invalid API key")]
    InvalidApiKey,

    #[error("Session expired or invalid")]
    SessionExpired,

    #[error("Invalid state parameter")]
    InvalidState,

    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Access denied")]
    Forbidden,

    #[error("Too many failed login attempts — try again later")]
    TooManyRequests,

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    /// Verifying a submitted password against its stored PHC string failed
    /// (the hashing task itself failed, not a wrong password).
    #[error("Password verification failed")]
    PasswordVerification(#[source] crate::auth::password::hash::PasswordHashError),

    /// Producing a PHC string for a new password failed.
    #[error("Password hashing failed")]
    PasswordHashing(#[source] crate::auth::password::hash::PasswordHashError),

    /// Resolving a GitHub App's bot identity failed. Boxed because the cause
    /// is itself an [`AuthError`] raised by the GitHub App service.
    #[error("Bot identity resolution failed")]
    BotIdentity(#[source] Box<AuthError>),

    /// An invariant this module synthesizes, with no underlying failure to
    /// carry — kept distinct from the wrapping variants above.
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::Unauthorized | Self::InvalidApiKey | Self::SessionExpired => {
                (StatusCode::UNAUTHORIZED, self.to_string())
            }
            Self::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            Self::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
            Self::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            Self::InvalidState => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::Internal(msg) => {
                error!(error = %msg, "Internal auth error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::PasswordVerification(_)
            | Self::PasswordHashing(_)
            | Self::BotIdentity(_)
            | Self::OAuth(_)
            | Self::Config(_)
            | Self::Database(_) => {
                // Log the full causal chain server-side exactly once, return a
                // generic message to the client.
                error!(error = %moatless_core::error::causal_chain(&self), "Internal auth error");
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
    use crate::auth::password::hash::PasswordHashError;

    async fn body_of(err: AuthError) -> (StatusCode, serde_json::Value) {
        let resp = err.into_response();
        let status = resp.status();
        let bytes = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    fn password_failures_keep_the_hashing_error_as_source() {
        let err = AuthError::PasswordVerification(PasswordHashError::Join);
        let source = std::error::Error::source(&err).expect("hashing error is the source");
        assert_eq!(source.to_string(), "hashing task failed to run");

        let err = AuthError::PasswordHashing(PasswordHashError::Hash("argon2 salt".into()));
        let source = std::error::Error::source(&err).expect("hashing error is the source");
        assert_eq!(source.to_string(), "password hashing failed: argon2 salt");
    }

    #[test]
    fn bot_identity_keeps_the_underlying_auth_failure_as_source() {
        let err = AuthError::BotIdentity(Box::new(AuthError::OAuth(
            "GitHub /app returned 401".to_string(),
        )));
        let source = std::error::Error::source(&err).expect("inner auth error is the source");
        assert_eq!(source.to_string(), "OAuth error: GitHub /app returned 401");
    }

    #[test]
    fn database_failures_keep_the_sqlx_error_as_source() {
        let err = AuthError::Database(sqlx::Error::RowNotFound);
        assert!(std::error::Error::source(&err).is_some());
    }

    #[tokio::test]
    async fn wrapped_failures_respond_500_without_cryptographic_or_sql_detail() {
        for err in [
            AuthError::PasswordHashing(PasswordHashError::Hash(
                "argon2id$v=19$m=19456 salt rejected".into(),
            )),
            AuthError::PasswordVerification(PasswordHashError::Hash("PHC parse".into())),
            AuthError::BotIdentity(Box::new(AuthError::Config(
                "GITHUB_APP_PRIVATE_KEY missing".into(),
            ))),
            AuthError::Database(sqlx::Error::RowNotFound),
        ] {
            let (status, json) = body_of(err).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(
                json,
                serde_json::json!({ "error": "Internal server error" })
            );
        }
    }
}
