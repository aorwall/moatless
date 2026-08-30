//! Task error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_types::access_control::AccessControlError;
use moatless_core::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("Task not found: {0}")]
    NotFound(String),

    #[error("Repository not found: {0}")]
    RepoNotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    /// A published store seam failed. Distinct from [`Self::Db`] only in where
    /// the failure was raised; both are an internal error to a caller.
    #[error("{0}")]
    Store(#[from] moatless_backend_core::store::StoreError),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("{0}")]
    Conflict(String),
}

impl From<moatless_backend_core::sandbox::SandboxOpError> for TaskError {
    /// The tasks-band lifecycle port error, mapped the rest of the way to an HTTP
    /// outcome. The sandbox-band implementation already classified the underlying
    /// `LifecycleError`; this preserves the three client-visible codes — a closed
    /// task is a 400, lock contention a 409, a missing task a 404 — and renders
    /// everything else as a 500.
    fn from(value: moatless_backend_core::sandbox::SandboxOpError) -> Self {
        use moatless_backend_core::sandbox::SandboxOpError as Op;
        match value {
            Op::NotFound(id) => Self::NotFound(id),
            Op::NotAllowed(msg) => Self::BadRequest(msg),
            Op::InProgress(msg) => Self::Conflict(msg),
            Op::Internal(msg) => Self::Internal(msg),
        }
    }
}

impl From<AccessControlError> for TaskError {
    fn from(value: AccessControlError) -> Self {
        match value {
            AccessControlError::BadRequest(msg) => Self::BadRequest(msg),
            AccessControlError::Forbidden(msg) => Self::Forbidden(msg),
        }
    }
}

impl IntoResponse for TaskError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            TaskError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            TaskError::RepoNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            TaskError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            TaskError::Db(e) => {
                tracing::error!(error = %e, "Database error in task handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            TaskError::Store(e) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(e),
                    "Store error in task handler"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            TaskError::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal error in task handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            TaskError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            TaskError::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
        };

        error_response(status, message)
    }
}
