/// Why the backend refused to admit an event — and, decisively, whether the
/// sandbox should send it again.
///
/// # Retry contract
///
/// This type is the **only** place `publish_event` builds a gRPC status, so the
/// sandbox's retry policy can be stated in one table:
///
/// | gRPC status | meaning | sandbox action |
/// |---|---|---|
/// | `OK` | the complete payload is durably admitted | drop the event; never resend |
/// | `UNAVAILABLE` | nothing was admitted | resend with the **same** `event_id` and `sequence` |
/// | `INVALID_ARGUMENT` | the request violates the wire contract | terminal; resending cannot succeed |
///
/// Every variant except [`AdmissionError::Invalid`] is retryable, so a new
/// failure mode is retryable unless it is deliberately made terminal. Callers
/// classify with [`AdmissionError::is_retryable`] rather than by matching
/// variants.
#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    /// Terminal: the request cannot be admitted as sent, no matter how often it
    /// is repeated. The message is backend-authored and safe to return.
    #[error("{0}")]
    Invalid(String),

    /// Retryable: a durability path this event kind requires is not configured.
    #[error("{component} is not configured")]
    NotConfigured { component: &'static str },

    /// Retryable: the durable write failed.
    #[error("failed to commit {what}")]
    CommitFailed {
        what: &'static str,
        #[source]
        source: sqlx::Error,
    },

    /// Retryable: this event's content only reaches its canonical table through
    /// the consumer path, and no event transport is configured — the outbox
    /// would spool a payload nothing can ever deliver.
    #[error("event transport is disabled and {what} has no durable substitute")]
    NoDeliveryPath { what: &'static str },

    /// Retryable: a concurrent request already owns admission for this event
    /// id. Answering `OK` would acknowledge an admission that may still fail;
    /// the retry converges on the winner's outcome.
    #[error("admission for this event id is already in flight")]
    AdmissionInFlight,
}

impl AdmissionError {
    /// Whether the sandbox should resend this event under the same id.
    pub fn is_retryable(&self) -> bool {
        !matches!(self, Self::Invalid(_))
    }
}
