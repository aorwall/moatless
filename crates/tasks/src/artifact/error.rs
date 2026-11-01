//! Artifact error types.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use super::rules::WriteOp;
use crate::collection::keys::InvalidRecord;

use moatless_core::error::{error_response, internal_error};

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    /// Also raised for an Artifact on a Task the caller cannot read, so the
    /// response does not tell a stranger it exists.
    #[error("Artifact not found: {0}")]
    NotFound(String),

    /// The caller can read the Task but not change it.
    #[error("Not allowed to change artifacts on task {0}")]
    Forbidden(String),

    /// The Artifact's record rules refuse a page this write. `prefix` is the
    /// rule that refused it, `None` when no rule covers `key`.
    #[error("{}", denial(*op, key, prefix.as_deref()))]
    PermissionDenied {
        op: WriteOp,
        key: String,
        prefix: Option<String>,
    },

    /// A page token that is missing, expired, or not signed here.
    #[error("Invalid or expired artifact page token")]
    Unauthorized,

    #[error("Invalid request: {0}")]
    BadRequest(String),

    /// Other writes kept changing the record a page's write was judged on.
    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("Artifact storage is not configured")]
    StorageNotConfigured,

    #[error("Object storage error")]
    Storage(#[from] moatless_s3::S3Error),

    #[error("Database error")]
    Db(#[from] sqlx::Error),

    /// Reading the Workspace or repository an attached collection belongs to.
    #[error("Store error")]
    Store(#[from] moatless_types::store::StoreError),

    /// Creating the Task a fork lands in; answers with the Task's own status.
    #[error(transparent)]
    Task(#[from] crate::task::error::TaskError),
}

impl From<InvalidRecord> for ArtifactError {
    fn from(error: InvalidRecord) -> Self {
        match error {
            InvalidRecord::Invalid(message) => Self::BadRequest(message),
            InvalidRecord::TooLarge(message) => Self::PayloadTooLarge(message),
        }
    }
}

fn denial(op: WriteOp, key: &str, prefix: Option<&str>) -> String {
    let op = op.as_str();
    match prefix {
        Some(prefix) => {
            format!("The rule for prefix {prefix:?} does not let this page {op} {key:?}")
        }
        None => format!("No record rule covers {key:?}, so this page cannot {op} it"),
    }
}

impl IntoResponse for ArtifactError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            // The Task's own refusal, such as a Workspace the caller cannot use.
            Self::Task(error) => return error.into_response(),
            Self::PermissionDenied { ref prefix, .. } => {
                // `code` and `prefix` are what the page script puts on the
                // error it rejects with; a page tells this refusal apart by them.
                let body = serde_json::json!({
                    "error": self.to_string(),
                    "code": "permission-denied",
                    "prefix": prefix,
                });
                return (StatusCode::FORBIDDEN, Json(body)).into_response();
            }
            Self::TaskNotFound(_) | Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            Self::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
            Self::StorageNotConfigured => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            Self::Storage(_) | Self::Db(_) | Self::Store(_) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    "Storage error in artifact handler"
                );
                internal_error()
            }
        };

        error_response(status, message)
    }
}
