use super::AdmissionError;
use tonic::Status;
use tracing::warn;

impl AdmissionError {
    /// The gRPC status for this failure, logging the causal chain exactly once.
    ///
    /// Redaction happens here, at the transport boundary: retryable failures
    /// collapse to one stable message so no SQL, connection or credential
    /// detail crosses the wire, while the full source chain is logged.
    pub fn into_status(self, task_id: &str, event_id: &str) -> Status {
        match self {
            Self::Invalid(message) => Status::invalid_argument(message),
            retryable => {
                warn!(
                    task_id,
                    event_id,
                    error = %retryable,
                    source = ?std::error::Error::source(&retryable),
                    "event not durably admitted; answering retryable"
                );
                Status::unavailable("event not durably admitted; retry with the same event_id")
            }
        }
    }
}
