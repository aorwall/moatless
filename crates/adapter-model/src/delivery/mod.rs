//! The contract an Adapter delivery is admitted and executed against.
//!
//! What an inbound delivery sets off cannot honestly finish on the HTTP
//! response path: routing an inbound message can create a task and provision a
//! sandbox, and a sender that waits three seconds for a 200 will not wait for
//! that. So an Adapter hands the work over durably before it acknowledges, and
//! this module is the vocabulary of that handover: the request an Adapter
//! submits ([`AdapterDeliveryRequest`]), what admission did with it
//! ([`Admission`], [`AdmitError`]), the record a handler receives
//! ([`AdapterDelivery`]), how a handler reports failure
//! ([`AdapterDeliveryFailure`]), and the registry that joins a `kind` to the
//! code that executes it ([`AdapterDeliveryHandler`],
//! [`AdapterDeliveryHandlers`]).
//!
//! **One sender delivery is one *or more* records.** The identity is
//! `(adapter, kind, event_id)`, so a delivery that fans into several kinds —
//! GitHub activity on an open pull request is the ordinary case — becomes one
//! record per kind, each retrying and failing on its own budget.
//!
//! # What the contract promises
//!
//! **When the sender is acknowledged.** After admission returns
//! [`Admission::Admitted`] or [`Admission::Duplicate`] — that is, after the
//! record is committed. An [`AdmitError`] must be surfaced to the sender as a
//! retryable failure so it redelivers; it must never be swallowed into a 200.
//!
//! **What is durable.** The whole `payload`, which must be everything the
//! handler needs to redo the delivery's work from scratch. Nothing in process
//! memory survives the crash this seam defends against, so a payload that
//! references in-memory state is not durable.
//!
//! **Retries and deduplication.** Deduplication is on `(adapter, kind,
//! event_id)` where `event_id` is the sender's delivery identifier: a
//! redelivery is a duplicate, not a second record. Execution is at-least-once,
//! so handlers must be idempotent.
//!
//! # What is not here
//!
//! The machinery that *keeps* the promise — the Postgres store, the supervised
//! worker, and the `AdapterDeliveryPolicy` that tunes their backoff, leases and
//! capacity — belongs to the band that runs it, and stays in
//! `moatless_loops::adapters::delivery`. A provider crate names this module to
//! implement the contract and never needs the machinery behind it.

mod handlers;
mod models;

pub use handlers::{AdapterDeliveryHandler, AdapterDeliveryHandlers};
pub use models::{
    AdapterDelivery, AdapterDeliveryFailure, AdapterDeliveryRequest, Admission, AdmitError,
};

use moatless_types::adapter::InboundDeliveryPayload;

use crate::error::{AdapterError, SharedAdapterError};

/// Admission failures reach the sender as "not accepted, send it again".
///
/// Both variants are the backend's problem, not the request's: nothing about
/// the delivery was wrong, so a 4xx would tell the sender to stop retrying
/// work that is still safe to retry. Capacity maps to `503` because the
/// condition is explicitly transient; a failed durable write maps to the
/// existing `500` path, which logs the cause once and returns no detail.
///
/// The store failure is carried across, not rebuilt: [`AdmitError::Store`]
/// already holds the store error the raise site named, so this arm hands the
/// same value to the same variant it always landed in.
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
/// Every sender handler ends up calling adapter runtime code that reports
/// [`AdapterError`], so the retryable/permanent split is decided once here
/// rather than once per sender. The question each arm answers is "could a
/// later attempt see a different answer?".
///
/// The split is deliberately asymmetric. Calling a transient failure permanent
/// abandons work the backend has already told a sender it owns; calling a
/// permanent failure transient only delays the same terminal record by the
/// attempt budget, and it still lands in `failed` with its cause. So anything
/// that is not clearly a rejected or missing input is retried — including
/// [`SharedAdapterError::Internal`], which the adapter runtimes still use for any
/// state they could not classify.
///
/// The causing error is kept as the source either way, so a terminal record's
/// `last_error` still names what actually failed.
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

/// Build the durable request for one inbound message, keyed by the sender's
/// own delivery id.
///
/// Admitting it is a separate call every caller makes for itself. That split is
/// deliberate: this function used to admit as well, and flattened the
/// `AdmitError` into an `AdapterError` on the way out (`.admit(..).await?`),
/// which made the shared "not admitted" ingress variant unreachable for every
/// Adapter that went through it. What is left here is the part worth sharing:
/// the payload shape and the one serialization error, spelled once instead of
/// at each of the three call sites.
pub fn inbound_delivery_request(
    adapter: &'static str,
    kind: &'static str,
    event_id: String,
    payload: InboundDeliveryPayload,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    let payload = serde_json::to_value(payload).map_err(|e| {
        SharedAdapterError::Internal(format!("Failed to serialize inbound delivery payload: {e}"))
    })?;
    Ok(AdapterDeliveryRequest {
        adapter,
        kind,
        event_id,
        payload,
    })
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

    /// Every variant's retry class is pinned, exhaustively, so a variant that
    /// later moves or is added cannot silently change whether delivery gives up.
    /// Wrongly-permanent is the expensive mistake — it abandons work the sender
    /// was told we owned — so a record ends early only when its input, or the
    /// thing it names, is wrong, or when redoing the identical work is provably
    /// futile.
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
            AdapterError::task_handoff("open a task from a provider event", sqlx::Error::RowNotFound),
            AdapterError::task_handoff("deliver a provider turn to a task", sqlx::Error::RowNotFound),
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
