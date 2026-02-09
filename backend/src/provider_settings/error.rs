//! Provider settings error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug)]
pub enum ProviderSettingsError {
    NotFound(String),
    BadRequest(String),
    Db(sqlx::Error),
    ProviderError(String),
}

impl IntoResponse for ProviderSettingsError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            Self::NotFound(m) => (StatusCode::NOT_FOUND, m),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::Db(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {e}"),
            ),
            Self::ProviderError(m) => (StatusCode::BAD_GATEWAY, m),
        };
        (status, msg).into_response()
    }
}

impl From<sqlx::Error> for ProviderSettingsError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}
