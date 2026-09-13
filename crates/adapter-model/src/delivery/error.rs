//! Delivery admission and execution failures.

use crate::error::{AdapterError, SharedAdapterError};
use moatless_types::store::StoreError;

/// Admission failed, so the sender must not be told the delivery was accepted.
#[derive(Debug, thiserror::Error)]
pub enum AdmitError {
    /// The unfinished backlog is at its configured ceiling; refusing keeps
    /// ingress bounded.
    ///
    /// `limit` is the configured capacity, not the observed count.
    #[error("Adapter delivery backlog is at capacity ({limit} unfinished records)")]
    AtCapacity { limit: i64 },
    /// The durable write itself failed. [`StoreError`] carries the driver error
    /// as its source, so this variant names no database driver itself. The
    /// cause is redacted at the transport boundary.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Why a handler could not complete an Adapter delivery.
///
/// `Retryable` spends one attempt; `Permanent` ends the record immediately.
/// Both keep the causing error as their source.
#[derive(Debug, thiserror::Error)]
pub enum AdapterDeliveryFailure {
    /// A transient failure — a timeout, a sender 5xx, a lost connection.
    /// Retried with backoff until the attempt budget runs out.
    #[error("retryable Adapter delivery failure")]
    Retryable(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The delivery can never succeed: a malformed payload, a deleted task, a
    /// revoked connection. Goes terminal on this attempt, no matter how much
    /// budget is left.
    #[error("permanent Adapter delivery failure")]
    Permanent(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl AdapterDeliveryFailure {
    /// Wrap a transient failure.
    pub fn retryable(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Retryable(Box::new(source))
    }

    /// Wrap a failure that no number of retries can fix.
    pub fn permanent(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Permanent(Box::new(source))
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }
}
/// Admission failures reach the sender as "not accepted, send it again", never
/// a 4xx: a `503` for capacity (transient), `500` for a failed durable write.
impl From<AdmitError> for AdapterError {
    fn from(error: AdmitError) -> Self {
        match error {
            AdmitError::AtCapacity { .. } => Self::Shared(SharedAdapterError::Overloaded),
            AdmitError::Store(source) => Self::Shared(SharedAdapterError::Store(source)),
        }
    }
}

/// Classify an adapter failure into the seam's retry policy.
///
/// Deliberately asymmetric: calling a transient failure permanent abandons work
/// the backend already told the sender it owns, so anything not clearly a
/// rejected or missing input is retried.
impl From<AdapterError> for AdapterDeliveryFailure {
    fn from(error: AdapterError) -> Self {
        match error {
            // Transient: resolving a credential failed (the token may be
            // refreshed or the installation repaired between attempts), or a
            // task-flow storage failure the call site already decided is worth
            // redelivering — a preflight refusal is handled there and never
            // reaches here.
            AdapterError::UpstreamCredential { .. }
            | AdapterError::TaskHandoff { .. }
            // Not a failure at all: another delivery won the creation claim for
            // this subject. The retry is the mechanism, not a recovery — it is
            // how this event reaches the task the winner made.
            | AdapterError::CreationClaimed { .. }
            | AdapterError::Store(_) => Self::retryable(error),
            // Transient: infrastructure, the upstream sender, our own capacity,
            // a message this process synthesized for a condition it could not
            // classify, or a provider we never reached / could not decode —
            // the latter two sit on the same 502 path as `AdapterApi` and a
            // later attempt can genuinely see a different answer.
            AdapterError::Shared(
                SharedAdapterError::Store(_)
                | SharedAdapterError::AdapterApi(_)
                | SharedAdapterError::Unauthorized(_)
                | SharedAdapterError::Overloaded
                | SharedAdapterError::AdapterNotConfigured(_)
                | SharedAdapterError::Internal(_)
                | SharedAdapterError::ProviderUnreachable { .. }
                | SharedAdapterError::ProviderResponseInvalid { .. },
            ) => Self::retryable(error),
            // Permanent: the input, or the thing it names, is wrong — redoing
            // the identical work cannot change that. `ConfigSerialization` is
            // the one arm where futility is provable rather than merely likely:
            // serializing a config we already hold is deterministic.
            AdapterError::Shared(
                SharedAdapterError::ConfigNotFound(_)
                | SharedAdapterError::NotFound(_)
                | SharedAdapterError::BadRequest(_)
                | SharedAdapterError::Conflict(_)
                | SharedAdapterError::SignatureInvalid
                | SharedAdapterError::AccessDenied(_)
                | SharedAdapterError::MultipleTargets { .. }
                | SharedAdapterError::ConfigSerialization(_),
            ) => Self::permanent(error),
        }
    }
}

// The retry-class and redaction tests need a stand-in cause and the renderer
// that redacts it: `postgres` gives them an `sqlx::Error`, `http` the
// `IntoResponse` impl. Both are on for this crate's own test targets through
// the self-referential dev-dependency.
#[cfg(all(test, feature = "http", feature = "postgres"))]
mod tests {
    use super::*;
    use moatless_types::store::StoreError;

    /// A failed durable write, shaped exactly as the admission path raises it:
    /// the operation named, the driver error kept as the source.
    fn failed_write() -> AdmitError {
        AdmitError::Store(StoreError::with_source(
            "persist the Adapter delivery record",
            sqlx::Error::PoolTimedOut,
        ))
    }

    /// Every variant's retry class is pinned, so a later change cannot silently
    /// flip whether delivery gives up. Wrongly-permanent is the expensive
    /// mistake: it abandons work the sender was told it owns.
    #[tokio::test]
    async fn every_adapter_error_pins_its_retry_class() {
        let retryable: Vec<AdapterError> = vec![
            SharedAdapterError::Store(StoreError::with_source(
                "read the connection",
                sqlx::Error::PoolTimedOut,
            ))
            .into(),
            SharedAdapterError::AdapterApi("502 from GitHub".into()).into(),
            SharedAdapterError::Unauthorized("token expired".into()).into(),
            SharedAdapterError::Overloaded.into(),
            SharedAdapterError::AdapterNotConfigured("github_pr".into()).into(),
            SharedAdapterError::Internal("failed to create task: db blip".into()).into(),
            SharedAdapterError::provider_unreachable(
                "chat.postMessage",
                crate::test_support::transport_error().await,
            )
            .into(),
            SharedAdapterError::provider_response_invalid(
                "Linear JSON",
                serde_json::from_str::<serde_json::Value>("{oops").unwrap_err(),
            )
            .into(),
            AdapterError::upstream_credential(
                "resolve the repository's GitHub credential",
                sqlx::Error::PoolClosed,
            ),
            AdapterError::upstream_credential(
                "resolve the GitHub token for user u-1",
                sqlx::Error::PoolClosed,
            ),
            AdapterError::task_handoff(
                "open a task from a provider event",
                sqlx::Error::RowNotFound,
            ),
            AdapterError::task_handoff(
                "deliver a provider turn to a task",
                sqlx::Error::RowNotFound,
            ),
        ];
        for error in retryable {
            let rendered = error.to_string();
            assert!(
                !AdapterDeliveryFailure::from(error).is_permanent(),
                "{rendered} must be retried"
            );
        }

        let permanent: Vec<AdapterError> = vec![
            SharedAdapterError::ConfigNotFound("connection".into()).into(),
            SharedAdapterError::NotFound("task".into()).into(),
            SharedAdapterError::BadRequest("missing repository.full_name".into()).into(),
            SharedAdapterError::Conflict("already linked".into()).into(),
            SharedAdapterError::SignatureInvalid.into(),
            SharedAdapterError::AccessDenied("not a member".into()).into(),
            SharedAdapterError::MultipleTargets { targets: vec![] }.into(),
            SharedAdapterError::ConfigSerialization(
                serde_json::from_str::<serde_json::Value>("{oops").unwrap_err(),
            )
            .into(),
        ];
        for error in permanent {
            let rendered = error.to_string();
            assert!(
                AdapterDeliveryFailure::from(error).is_permanent(),
                "{rendered} must go terminal"
            );
        }
    }

    /// A terminal record has to say what actually failed, not just that
    /// something did.
    #[test]
    fn classification_keeps_the_adapter_error_as_the_cause() {
        let failure = AdapterDeliveryFailure::from(AdapterError::from(
            SharedAdapterError::BadRequest("bad payload".into()),
        ));
        let source = std::error::Error::source(&failure).expect("the adapter error is retained");
        assert!(source.to_string().contains("bad payload"));
    }

    #[test]
    fn the_admission_failure_keeps_its_cause() {
        let error = failed_write();
        let source = std::error::Error::source(&error).expect("the sqlx error is retained");
        assert!(
            source.to_string().contains("pool timed out"),
            "the driver error stays one link down, not flattened into the store \
             error's context: {source}"
        );
    }

    /// The rendered halves of the admission contract: what a refused or failed
    /// admission puts on the wire.
    mod rendered {
        use super::*;
        use axum::response::IntoResponse;
        use http_body_util::BodyExt;

        async fn response_of(error: AdmitError) -> (axum::http::StatusCode, String) {
            let response = AdapterError::from(error).into_response();
            let status = response.status();
            let body = response
                .into_body()
                .collect()
                .await
                .map(|b| String::from_utf8_lossy(&b.to_bytes()).into_owned())
                .unwrap_or_default();
            (status, body)
        }

        #[tokio::test]
        async fn a_full_backlog_asks_the_sender_to_retry() {
            let (status, body) = response_of(AdmitError::AtCapacity { limit: 10 }).await;
            assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
            assert!(
                body.contains("Service temporarily unavailable"),
                "the client sees a stable message: {body}"
            );
            assert!(
                !body.contains("adapter_delivery") && !body.contains("backlog"),
                "capacity internals stay out of the response: {body}"
            );
        }

        #[tokio::test]
        async fn a_failed_durable_write_returns_a_redacted_500() {
            let (status, body) = response_of(failed_write()).await;
            assert_eq!(status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(body, r#"{"error":"Internal server error"}"#);
        }
    }
}
