//! Durable admission for every inbound Adapter delivery.
//!
//! What an inbound delivery sets off cannot honestly finish on the HTTP
//! response path: routing an inbound message can create a task and provision a
//! sandbox, and a sender that waits three seconds for a 200 will not wait for
//! that. Spawning it and returning 200 anyway makes the acknowledgment a lie —
//! the sender believes the delivery is accepted, and a deploy, an OOM kill, or
//! a panic loses it with no trace and no redelivery.
//!
//! This module is the one seam that makes such an acknowledgment true. An
//! Adapter delivery is written to `adapter_delivery` inside the request
//! handler, keyed by the sender's own delivery identifier; the supervised
//! `adapter-delivery-worker` claims it, executes it, and drives it to a
//! terminal state. Answering a sender with success means the delivery is
//! durably owned, not that a task was spawned.
//!
//! **One sender delivery is one *or more* records.** The identity is
//! `(adapter, kind, event_id)`, so a delivery that fans into several kinds —
//! GitHub activity on an open pull request is the ordinary case — becomes one
//! record per kind, each retrying and failing on its own budget. The sender's
//! own interface counts that same event as one; this is the stated cost of
//! naming the record for what it is for.
//!
//! # Admission and overload semantics
//!
//! **When the sender is acknowledged.** After [`AdapterDeliveryStore::admit`]
//! returns [`Admission::Admitted`] or [`Admission::Duplicate`] — that is, after
//! the record is committed. An [`AdmitError`] must be surfaced to the sender
//! as a retryable failure so it redelivers; it must never be swallowed into a
//! 200.
//!
//! **What is durable.** The whole `payload`, which must be everything the
//! handler needs to redo the delivery's work from scratch. Nothing in process
//! memory survives the crash this table defends against, so a payload that
//! references in-memory state is not durable.
//!
//! **Retries and deduplication.** Deduplication is a unique index on
//! `(adapter, kind, event_id)` where `event_id` is the sender's delivery
//! identifier: a redelivery is a duplicate, not a second record. Execution is
//! at-least-once, so handlers must be idempotent. A failed attempt is retried
//! with exponential backoff up to [`AdapterDeliveryPolicy::max_attempts`]; the
//! last attempt, or any failure the handler classifies as
//! [`AdapterDeliveryFailure::Permanent`], makes the record terminal (`failed`)
//! with its cause retained in `last_error`, an `error` log, and a `terminal`
//! metric increment.
//!
//! **What happens at capacity.** Admission is bounded by
//! [`AdapterDeliveryPolicy::backlog_capacity`] unfinished records and refuses
//! with [`AdmitError::AtCapacity`] past it. Refusing is the point: the sender
//! retries a delivery it still owns, instead of the backend accepting a
//! delivery it has no capacity to execute. The bound is a stable ceiling rather
//! than an exact cutoff — concurrent admissions can overshoot it by at most the
//! number in flight, and never ratchet past that.
//! [`AdapterDeliveryPolicy::backlog_capacity`] states the arithmetic and why an
//! exact cutoff was not bought.
//!
//! **How shutdown drains.** On root cancellation the worker stops claiming new
//! batches and waits for the batch in flight — but only for as long as the
//! process itself waits. The root supervisor bounded-joins its tasks for
//! `SHUTDOWN_JOIN_DEADLINE` (20s) and then returns, naming whatever is still
//! running, while [`AdapterDeliveryPolicy::execution_timeout`] allows a single
//! execution 2 minutes. A deploy that catches a long-running batch therefore
//! exits with handlers mid-flight, and logs `adapter-delivery-worker` as a
//! straggler when it does; that warning is the expected shape of this
//! interaction, not a fault.
//!
//! Correctness does not rest on the drain. Records still `claimed` when the
//! process dies are recovered by the next run of the same owner, or by claim
//! lease expiry if that owner never returns. Shutdown delays an admitted
//! delivery; it never loses it — and that, rather than a clean drain, is the
//! guarantee.
//!
//! # What admitting a delivery costs its Adapter
//!
//! Every Adapter is on this seam: Slack, Telegram, and the generic Webhook
//! Adapter route inbound messages through
//! [`crate::adapters::inbound_delivery`], and GitHub and Linear register their
//! own kinds. Nothing inbound runs on a response path any more. The two
//! properties below are what admitting gives up in exchange for durability, and
//! every Adapter has to answer them rather than discover them:
//!
//! **Per-connection webhook outcome telemetry.** The metric here is only
//! `{adapter, kind, outcome}` and is deliberately low-cardinality, whereas
//! [`record_webhook`](crate::adapters::metrics) carries `event_type`,
//! `loop_id`, and `connection_id` — which is what makes a single broken
//! Adapter connection visible. The seam's metric is not a replacement: a
//! handler must keep emitting its own webhook outcome metric, or per-connection
//! failure visibility is lost. The chat handler does, from inside `execute`,
//! with the labels carried in its payload for that sole purpose.
//!
//! **Execution start latency.** A spawn starts in microseconds. An admitted
//! delivery waits for the next poll, so the path trades a bounded scheduling
//! delay for durability. [`AdapterDeliveryPolicy::poll_interval`] is set for
//! interactive work rather than for batch throughput for exactly this reason.
//!
//! **In-process deduplication no Adapter needs any more.** Every Adapter once
//! kept a five-minute, per-process cache of handled event ids in front of
//! routing; all of them are gone. Keyed on the same identifier this table is
//! keyed on, such a cache sits *in front of* a fallible admission, so a
//! redelivery whose admission failed would be answered with a 200 and nothing
//! durable behind it — the precise acknowledgment lie this module exists to
//! remove. What the caches also covered, and the unique index does not, is
//! repeat *parse-time* side effects on a redelivery; that consequence is stated
//! where each Adapter parses.
//!
//! # What does not belong here
//!
//! Best-effort outbound activity notifications and API-key usage timestamps
//! stay off this path. They are allowed to be lost, and admitting them would
//! make every webhook slower and this table larger for no correctness gain.

mod handlers;
mod metrics;
mod models;
mod store;
mod typed;
mod worker;

pub use handlers::{AdapterDeliveryHandler, AdapterDeliveryHandlers};
pub use metrics::describe_adapter_delivery_metrics;
pub use models::{
    AdapterDelivery, AdapterDeliveryFailure, AdapterDeliveryPolicy, AdapterDeliveryRequest,
    Admission, AdmitError,
};
pub use store::AdapterDeliveryStore;
pub use typed::{ConnectionDeliveryPayload, register_connection_delivery, register_typed_delivery};
pub use worker::spawn_adapter_delivery_worker;

use crate::adapters::error::{AdapterError, SharedAdapterError};

/// Admission failures reach the sender as "not accepted, send it again".
///
/// Both variants are the backend's problem, not the request's: nothing about
/// the delivery was wrong, so a 4xx would tell the sender to stop retrying
/// work that is still safe to retry. Capacity maps to `503` because the
/// condition is explicitly transient; a failed durable write maps to the
/// existing `500` path, which logs the cause once and returns no detail.
impl From<AdmitError> for AdapterError {
    fn from(error: AdmitError) -> Self {
        match error {
            AdmitError::AtCapacity { .. } => Self::Shared(SharedAdapterError::Overloaded),
            AdmitError::Store(source) => Self::Shared(SharedAdapterError::Db(source)),
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
            // redelivering — `TaskCreation`'s preflight rejections are handled
            // there and never reach here.
            AdapterError::RepositoryCredential { .. }
            | AdapterError::UserCredential { .. }
            | AdapterError::TaskCreation(_)
            | AdapterError::MessageDelivery(_)
            | AdapterError::Store(_) => Self::retryable(error),
            // Transient: infrastructure, the upstream sender, our own capacity,
            // a message this process synthesized for a condition it could not
            // classify, or a provider we never reached / could not decode —
            // the latter two sit on the same 502 path as `AdapterApi` and a
            // later attempt can genuinely see a different answer.
            AdapterError::Shared(
                SharedAdapterError::Db(_)
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

#[cfg(test)]
mod tests {
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    use super::*;
    use crate::adapters::test_support::transport_error;

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
        let (status, body) = response_of(AdmitError::Store(sqlx::Error::PoolTimedOut)).await;
        assert_eq!(status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
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
            SharedAdapterError::Db(sqlx::Error::PoolTimedOut).into(),
            SharedAdapterError::AdapterApi("502 from GitHub".into()).into(),
            SharedAdapterError::Unauthorized("token expired".into()).into(),
            SharedAdapterError::Overloaded.into(),
            SharedAdapterError::AdapterNotConfigured("github_pr".into()).into(),
            SharedAdapterError::Internal("failed to create task: db blip".into()).into(),
            SharedAdapterError::provider_unreachable("chat.postMessage", transport_error().await)
                .into(),
            SharedAdapterError::provider_response_invalid(
                "Linear JSON",
                serde_json::from_str::<serde_json::Value>("{oops").unwrap_err(),
            )
            .into(),
            AdapterError::RepositoryCredential {
                source: moatless_workspaces::repository::error::RepositoryError::Db(
                    sqlx::Error::PoolClosed,
                ),
            },
            AdapterError::UserCredential {
                user_id: "u-1".into(),
                source: moatless_identity::auth::error::AuthError::Forbidden,
            },
            AdapterError::TaskCreation(moatless_tasks::task::creation::TaskCreationError::Db(
                sqlx::Error::RowNotFound,
            )),
            AdapterError::MessageDelivery(moatless_tasks::message::error::MessageError::Db(
                sqlx::Error::RowNotFound,
            )),
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
        let error = AdmitError::Store(sqlx::Error::PoolTimedOut);
        let source = std::error::Error::source(&error).expect("the sqlx error is retained");
        assert!(source.to_string().contains("pool timed out"));
    }
}
