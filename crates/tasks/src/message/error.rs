//! Message error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("Message not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Task is closed: {0}")]
    TaskClosed(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    /// The external-event-id dedup gate could not be evaluated, so the send
    /// cannot tell a duplicate delivery from a first one.
    #[error("Duplicate-message check failed")]
    DedupCheck(#[source] sqlx::Error),

    /// Appending the message row (and its contexts/blocks) failed.
    #[error("Message could not be persisted")]
    PersistMessage(#[source] sqlx::Error),

    /// Appending the synthetic tool-response message failed.
    #[error("Tool response could not be persisted")]
    PersistToolResponse(#[source] sqlx::Error),

    /// The message was stored but advancing the task's conversation clock
    /// failed, so the wake input would be stale.
    #[error("Task conversation clock could not be advanced")]
    AdvanceTaskClock(#[source] sqlx::Error),

    /// The attached files could not be looked up, so the send cannot tell an
    /// id with no upload behind it from one it failed to read.
    #[error("Attachment metadata could not be read")]
    AttachmentMetadata(#[source] moatless_core::file_storage::FileStorageError),

    /// An invariant this module synthesizes, with no underlying failure to
    /// carry — kept distinct from the wrapping variants above.
    #[error("Internal error: {0}")]
    Internal(String),
}

impl MessageError {
    /// Whether the send failed with nothing of it written down.
    ///
    /// A Task deleted after its message persisted orphans it: `task_messages`
    /// has no foreign key to `tasks`. Only a variant raised after the append
    /// answers `false`.
    pub(crate) fn wrote_nothing(&self) -> bool {
        match self {
            // Raised before `MessageDao::append`, or by it — and its single
            // transaction commits only once every row has landed.
            MessageError::NotFound(_)
            | MessageError::BadRequest(_)
            | MessageError::TaskClosed(_)
            | MessageError::DedupCheck(_)
            | MessageError::AttachmentMetadata(_)
            | MessageError::PersistMessage(_)
            | MessageError::PersistToolResponse(_) => true,
            // The message row is committed by the time the clock advance runs,
            // and these two name no point in the sequence at all.
            MessageError::AdvanceTaskClock(_) | MessageError::Db(_) | MessageError::Internal(_) => {
                false
            }
        }
    }
}

impl IntoResponse for MessageError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            MessageError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            MessageError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            MessageError::TaskClosed(_) => (StatusCode::CONFLICT, self.to_string()),
            MessageError::Db(_)
            | MessageError::DedupCheck(_)
            | MessageError::PersistMessage(_)
            | MessageError::PersistToolResponse(_)
            | MessageError::AdvanceTaskClock(_)
            | MessageError::AttachmentMetadata(_) => {
                // One log line at the boundary carries the whole chain,
                // including the SQL error the client must never see.
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    "Database error in message handler"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            MessageError::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal error in message handler");
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

    /// A `sqlx::Error` whose `Display` looks like leaked SQL, so the snapshot
    /// below proves the body is built from the redacted message and never the
    /// error itself.
    fn sql_error() -> sqlx::Error {
        sqlx::Error::Protocol(
            "INSERT INTO task_messages (task_id, content) VALUES ($1, $2)".to_string(),
        )
    }

    async fn body_of(err: MessageError) -> (StatusCode, serde_json::Value) {
        let resp = err.into_response();
        let status = resp.status();
        let bytes = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    fn wrapped_send_failures_keep_the_sql_error_as_source() {
        for err in [
            MessageError::DedupCheck(sql_error()),
            MessageError::PersistMessage(sql_error()),
            MessageError::PersistToolResponse(sql_error()),
            MessageError::AdvanceTaskClock(sql_error()),
        ] {
            let source = std::error::Error::source(&err).expect("sqlx error is the source");
            assert!(
                source.to_string().contains("INSERT INTO task_messages"),
                "source lost for {err:?}"
            );
        }
    }

    #[test]
    fn a_refusal_wrote_nothing_and_a_failed_clock_advance_did_not() {
        assert!(MessageError::BadRequest("no uploaded file".to_string()).wrote_nothing());
        assert!(MessageError::TaskClosed("task-1".to_string()).wrote_nothing());
        // The append's own transaction rolled back.
        assert!(MessageError::PersistMessage(sql_error()).wrote_nothing());
        // The message row is already committed by the time this one is raised.
        assert!(!MessageError::AdvanceTaskClock(sql_error()).wrote_nothing());
    }

    #[test]
    fn synthesized_internal_messages_have_no_source() {
        let err = MessageError::Internal("no adapter registered".to_string());
        assert!(std::error::Error::source(&err).is_none());
    }

    #[tokio::test]
    async fn wrapped_send_failures_respond_500_without_sql_detail() {
        for err in [
            MessageError::DedupCheck(sql_error()),
            MessageError::PersistMessage(sql_error()),
            MessageError::PersistToolResponse(sql_error()),
            MessageError::AdvanceTaskClock(sql_error()),
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
