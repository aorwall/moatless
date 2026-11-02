//! Task error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::{error_response, internal_error};
use moatless_types::access_control::AccessControlError;

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
    Store(#[from] moatless_types::store::StoreError),

    #[error("Internal error: {0}")]
    Internal(String),

    /// The Task's first message could not be delivered. Creation undoes its
    /// insert before raising this, so it answers for a Task that no longer
    /// exists — and answers with the send's own status, not a blanket 500.
    #[error("{0}")]
    FirstMessage(#[source] crate::message::error::MessageError),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// The Task's own preflight refused to start it. The message is written
    /// for the person who created the Task, so every surface may show it.
    #[error("Forbidden: {0}")]
    Preflight(String),

    /// [`Self::Preflight`] for a check the git host rate-limited: temporary,
    /// so HTTP answers 503.
    #[error("{0}")]
    PreflightThrottled(String),

    #[error("{0}")]
    Conflict(String),
}

impl From<moatless_ports::sandbox::SandboxOpError> for TaskError {
    /// The tasks-band lifecycle port error, mapped to an HTTP outcome: a closed
    /// task is 400, lock contention 409, a missing task 404, and everything
    /// else the sandbox band classified renders 500.
    fn from(value: moatless_ports::sandbox::SandboxOpError) -> Self {
        use moatless_ports::sandbox::SandboxOpError as Op;
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
        let text = self.to_string();
        let (status, message) = match self {
            // The send classifies its own failures, so a refused attachment
            // answers 400 with what was wrong rather than a 500 naming nothing.
            TaskError::FirstMessage(e) => return e.into_response(),
            TaskError::NotFound(_) => (StatusCode::NOT_FOUND, text),
            TaskError::RepoNotFound(_) => (StatusCode::NOT_FOUND, text),
            TaskError::BadRequest(_) => (StatusCode::BAD_REQUEST, text),
            TaskError::Db(e) => {
                tracing::error!(error = %e, "Database error in task handler");
                internal_error()
            }
            TaskError::Store(e) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&e),
                    "Store error in task handler"
                );
                internal_error()
            }
            TaskError::Internal(msg) => {
                tracing::error!(error = %msg, "Internal error in task handler");
                internal_error()
            }
            TaskError::Forbidden(_) | TaskError::Preflight(_) => (StatusCode::FORBIDDEN, text),
            TaskError::PreflightThrottled(_) => (StatusCode::SERVICE_UNAVAILABLE, text),
            TaskError::Conflict(_) => (StatusCode::CONFLICT, text),
        };

        error_response(status, message)
    }
}
