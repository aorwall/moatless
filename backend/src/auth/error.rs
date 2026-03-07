//! Auth error types with axum IntoResponse.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use tracing::error;

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

    #[error("Not found: {0}")]
    NotFound(String),

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
            Self::InvalidState => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::OAuth(_) | Self::Config(_) | Self::Database(_) => {
                // Log the full error server-side, return generic message to client
                error!(error = %self, "Internal auth error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
