//! Feedback error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_response;

/// A stored `feedback` row holds `source` and `surface` as text. A value
/// outside the known set means the row was written by something other than
/// this API, and the row cannot be projected onto the response contract.
#[derive(Debug, thiserror::Error)]
#[error("feedback row column '{column}' holds unknown value {value:?}")]
pub struct UnknownFeedbackDiscriminant {
    pub column: &'static str,
    pub value: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FeedbackError {
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    /// A persisted feedback row could not be projected onto the response
    /// contract. Retains the rejected discriminant as its source.
    #[error("Stored feedback row is not representable")]
    UnreadableRow(#[from] UnknownFeedbackDiscriminant),

    /// An invariant this module synthesizes, with no underlying failure to
    /// carry — kept distinct from the wrapping variants above.
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),
}

impl IntoResponse for FeedbackError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::TaskNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Db(_) | Self::UnreadableRow(_) => {
                tracing::error!(
                    error = %crate::error::causal_chain(&self),
                    "Database error in feedback handler"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::Internal(msg) => {
                tracing::error!(error = %msg, "Internal error in feedback handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
        };

        error_response(status, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_row_keeps_the_rejected_discriminant_as_source() {
        let err = FeedbackError::from(UnknownFeedbackDiscriminant {
            column: "surface",
            value: "preview_faliure".to_string(),
        });
        let source = std::error::Error::source(&err).expect("discriminant error is the source");
        assert_eq!(
            source.to_string(),
            "feedback row column 'surface' holds unknown value \"preview_faliure\""
        );
    }

    #[test]
    fn synthesized_internal_messages_have_no_source() {
        let err = FeedbackError::Internal("feedback repository not wired".to_string());
        assert!(std::error::Error::source(&err).is_none());
    }

    #[tokio::test]
    async fn wrapped_failures_respond_500_without_row_or_sql_detail() {
        for err in [
            FeedbackError::from(UnknownFeedbackDiscriminant {
                column: "source",
                value: "robot".to_string(),
            }),
            FeedbackError::Db(sqlx::Error::Protocol(
                "SELECT * FROM feedback WHERE task_id = $1".to_string(),
            )),
        ] {
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
}
