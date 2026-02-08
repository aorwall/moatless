//! Figma error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum FigmaError {
    #[error("Import not found: {0}")]
    NotFound(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Figma API error: {0}")]
    ApiError(String),
}

impl IntoResponse for FigmaError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            FigmaError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            FigmaError::ParseError(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            FigmaError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            FigmaError::ApiError(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
