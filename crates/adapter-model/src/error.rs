//! The Adapter error space.
//!
//! [`SharedAdapterError`] is the band-local half: variants naming nothing above
//! this crate. [`AdapterError`] wraps it and adds variants that carry a failure
//! from another band, plus the `IntoResponse` renderer over both.
//!
//! Each cross-band variant boxes its source rather than naming the other band's
//! error type — `IntoResponse` only picks a status class and logs the chain, so
//! naming it would buy a dependency and no behaviour. The renderer sits behind
//! the `http` feature: what a failure *is* belongs to the kernel, what it looks
//! like on the wire belongs to whoever answers the request.

use moatless_types::access_control::AccessControlError;
use moatless_types::store::StoreError;

#[cfg(feature = "http")]
use axum::Json;
#[cfg(feature = "http")]
use axum::http::StatusCode;
#[cfg(feature = "http")]
use axum::response::{IntoResponse, Response};
#[cfg(feature = "http")]
use moatless_api_schemas::bindings::{MultipleTargetsErrorBody, ReplyTargetDescriptor};
#[cfg(feature = "http")]
use moatless_core::error::error_response;

/// One outbound destination a caller can select explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyTarget {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub flag_hint: String,
}

/// Why a provider's response body could not be turned into a value we can use.
///
/// Adapters decode either straight off the wire (`Response::json`) or from a
/// buffered body (`serde_json::from_str`); both stay reachable through
/// [`std::error::Error::source`].
#[derive(Debug, thiserror::Error)]
pub enum ProviderDecodeError {
    /// The HTTP client could not read or decode the body.
    #[error("{0}")]
    Body(#[from] reqwest::Error),

    /// The body was read, but it is not the JSON we expect.
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SharedAdapterError {
    #[error("Adapter config not found: {0}")]
    ConfigNotFound(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Adapter not configured: {0}")]
    AdapterNotConfigured(String),

    #[error("Signature verification failed")]
    SignatureInvalid,

    /// A read or write through the connection store failed. The store already
    /// names the operation and carries its own source.
    #[error(transparent)]
    Store(#[from] StoreError),

    /// The provider answered and rejected the call — a non-success status, or
    /// an `ok: false` / `errors` payload. Synthesized from the provider's own
    /// words, so there is no underlying Rust error to carry.
    #[error("Adapter API error: {0}")]
    AdapterApi(String),

    /// We never got an answer from the provider: DNS, TLS, connect, timeout, or
    /// a body that died mid-read. Distinct from
    /// [`SharedAdapterError::AdapterApi`] — callers telling "provider rejected
    /// us" from "we could not reach it" match on this.
    #[error("Adapter API error: {operation} failed: {source}")]
    ProviderUnreachable {
        /// The provider call that failed, e.g. `chat.postMessage`.
        operation: String,
        #[source]
        source: reqwest::Error,
    },

    /// The provider answered, but its payload could not be decoded.
    #[error("Adapter API error: {operation} parse error: {source}")]
    ProviderResponseInvalid {
        /// The provider call whose response could not be decoded.
        operation: String,
        #[source]
        source: ProviderDecodeError,
    },

    #[error("Adapter API error: {0}")]
    Unauthorized(String),

    #[error("Access denied: {0}")]
    AccessDenied(String),

    /// Returned by the no-flag send endpoint when a task has more than one
    /// reply target — the caller must pick one with `--thread`, `--binding`, or
    /// `--pr`.
    #[error("Task has multiple reply targets; pick one explicitly")]
    MultipleTargets { targets: Vec<ReplyTarget> },

    /// An adapter config could not be serialized back to JSON for persistence.
    #[error("Failed to serialize adapter config")]
    ConfigSerialization(#[source] serde_json::Error),

    /// The backend cannot take on any more Adapter deliveries right now.
    /// Distinct from `Internal` because it is explicitly transient: the sender
    /// still owns the delivery and should send it again.
    #[error("Service temporarily unavailable")]
    Overloaded,

    /// A message the adapter layer synthesizes itself — an impossible state, a
    /// downcast that cannot fail, an unsupported capability. There is no
    /// underlying error to carry; failures that wrap one use a variant above.
    #[error("Internal error: {0}")]
    Internal(String),
}

impl SharedAdapterError {
    /// The provider call never completed — keep the transport failure as
    /// source.
    pub fn provider_unreachable(operation: impl Into<String>, source: reqwest::Error) -> Self {
        Self::ProviderUnreachable {
            operation: operation.into(),
            source,
        }
    }

    /// The provider answered but we could not decode it — keep the decode
    /// failure as source.
    pub fn provider_response_invalid(
        operation: impl Into<String>,
        source: impl Into<ProviderDecodeError>,
    ) -> Self {
        Self::ProviderResponseInvalid {
            operation: operation.into(),
            source: source.into(),
        }
    }
}

/// The boxed source the two cross-band variants carry.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// A credential the Adapter needed could not be resolved — a repository's
    /// GitHub credential, or a user's token for an outbound call. Stays on the
    /// provider-facing 502 path.
    #[error("Adapter API error: {context}")]
    UpstreamCredential {
        context: String,
        #[source]
        source: BoxError,
    },

    /// Handing work to the task band failed: opening a Task from a provider
    /// event, or delivering an inbound turn onto one. Preflight refusals are
    /// handled at the call site and never reach here.
    #[error("{context}")]
    TaskHandoff {
        context: String,
        #[source]
        source: BoxError,
    },

    /// Another delivery is creating the task for this subject right now.
    ///
    /// Not a fault: the claim lets exactly one racing delivery create, and
    /// retrying is the point — by the next attempt the winner's binding exists
    /// and the message routes to it, or the event-receipt table drops it as a
    /// duplicate. Carries no source since nothing failed.
    #[error("Another delivery is creating the task for {subject_kind}:{subject_key}")]
    CreationClaimed {
        subject_kind: String,
        subject_key: String,
    },

    /// A read or write through a published store seam failed. The port already
    /// names the operation and carries its own source.
    #[error(transparent)]
    Store(#[from] StoreError),

    /// The band-local adapter vocabulary — the sixteen variants that name
    /// nothing above this crate.
    #[error(transparent)]
    Shared(#[from] SharedAdapterError),
}

impl AdapterError {
    /// A credential the Adapter needed could not be resolved.
    pub fn upstream_credential(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::UpstreamCredential {
            context: context.into(),
            source: Box::new(source),
        }
    }

    /// Handing work to the task band failed.
    pub fn task_handoff(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::TaskHandoff {
            context: context.into(),
            source: Box::new(source),
        }
    }
}

/// The bands above this crate own tables and read them straight through `sqlx`;
/// a failure there has no context narrower than the query that raised it.
/// Behind the same `postgres` gate as the connection store, so a driver-less
/// consumer has no `sqlx::Error` to convert.
#[cfg(feature = "postgres")]
impl From<sqlx::Error> for AdapterError {
    fn from(value: sqlx::Error) -> Self {
        Self::Shared(SharedAdapterError::Store(StoreError::with_source(
            "a Loop or Binding store call",
            value,
        )))
    }
}

impl From<AccessControlError> for AdapterError {
    fn from(value: AccessControlError) -> Self {
        match value {
            AccessControlError::BadRequest(msg) => {
                Self::Shared(SharedAdapterError::BadRequest(msg))
            }
            AccessControlError::Forbidden(msg) => {
                Self::Shared(SharedAdapterError::AccessDenied(msg))
            }
        }
    }
}

#[cfg(feature = "http")]
impl IntoResponse for AdapterError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AdapterError::UpstreamCredential { .. } => (StatusCode::BAD_GATEWAY, self.to_string()),
            AdapterError::TaskHandoff { context, .. } => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    context = %context,
                    "Handing work to the task band failed"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AdapterError::CreationClaimed { .. } => {
                // 409 rather than 5xx: the request is well-formed and the
                // system is behaving. A sender that redelivers on conflict
                // gets exactly the retry this wants.
                (StatusCode::CONFLICT, self.to_string())
            }
            AdapterError::Store(e) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(e),
                    "Store error in adapter handler"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AdapterError::Shared(inner) => match inner {
                SharedAdapterError::ConfigNotFound(_) | SharedAdapterError::NotFound(_) => {
                    (StatusCode::NOT_FOUND, inner.to_string())
                }
                SharedAdapterError::BadRequest(_) => (StatusCode::BAD_REQUEST, inner.to_string()),
                SharedAdapterError::Conflict(_) => (StatusCode::CONFLICT, inner.to_string()),
                SharedAdapterError::AdapterNotConfigured(_) => {
                    (StatusCode::BAD_REQUEST, inner.to_string())
                }
                SharedAdapterError::SignatureInvalid => {
                    (StatusCode::UNAUTHORIZED, inner.to_string())
                }
                SharedAdapterError::Store(e) => {
                    tracing::error!(
                        error = %moatless_core::error::causal_chain(e),
                        "Store error in adapter handler"
                    );
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error".to_string(),
                    )
                }
                SharedAdapterError::AccessDenied(_) => (StatusCode::FORBIDDEN, inner.to_string()),
                SharedAdapterError::AdapterApi(_)
                | SharedAdapterError::Unauthorized(_)
                | SharedAdapterError::ProviderUnreachable { .. }
                | SharedAdapterError::ProviderResponseInvalid { .. } => {
                    (StatusCode::BAD_GATEWAY, inner.to_string())
                }
                SharedAdapterError::MultipleTargets { targets } => {
                    let body = MultipleTargetsErrorBody {
                        error: inner.to_string(),
                        targets: targets
                            .iter()
                            .map(|target| ReplyTargetDescriptor {
                                kind: target.kind.clone(),
                                id: target.id.clone(),
                                label: target.label.clone(),
                                flag_hint: target.flag_hint.clone(),
                            })
                            .collect(),
                        hint: "Use --binding <binding_id> to pick one.".to_string(),
                    };
                    return (StatusCode::BAD_REQUEST, Json(body)).into_response();
                }
                SharedAdapterError::ConfigSerialization(e) => {
                    tracing::error!(error = %e, "Failed to serialize adapter config");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error".to_string(),
                    )
                }
                SharedAdapterError::Overloaded => {
                    tracing::warn!("Adapter delivery admission refused: backlog at capacity");
                    (StatusCode::SERVICE_UNAVAILABLE, inner.to_string())
                }
                SharedAdapterError::Internal(msg) => {
                    tracing::error!(error = %msg, "Internal error in adapter handler");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error".to_string(),
                    )
                }
            },
        };

        error_response(status, message)
    }
}

// Gated on the two features the assertions need: `http` puts `IntoResponse` in
// scope, `postgres` gives them an `sqlx::Error` to use as a stand-in cause. Both
// are on for every test target of this crate — the dev-dependency on itself in
// Cargo.toml enables them — so this is a compile gate, not a way for the suite
// to go quiet when the crate is tested alone.
#[cfg(all(test, feature = "http", feature = "postgres"))]
mod tests {
    use std::error::Error as _;

    use super::*;
    use crate::test_support::{response_snapshot, source_chain, transport_error};

    /// `Store` carries its cause, so the driver error a lookup failed on stays
    /// reachable through `Error::source` while the response stays redacted.
    #[tokio::test]
    async fn a_store_lookup_keeps_its_source_and_stays_redacted() {
        let err: AdapterError = SharedAdapterError::Store(StoreError::with_source(
            "read the connection",
            sqlx::Error::RowNotFound,
        ))
        .into();

        assert!(
            err.source()
                .and_then(|s| s.downcast_ref::<sqlx::Error>())
                .is_some()
        );

        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
    }

    #[tokio::test]
    async fn provider_unreachable_keeps_reqwest_source() {
        let err: AdapterError =
            SharedAdapterError::provider_unreachable("chat.postMessage", transport_error().await)
                .into();

        let source = err.source().expect("transport failure must stay reachable");
        assert!(source.downcast_ref::<reqwest::Error>().is_some());
        assert!(
            err.to_string()
                .starts_with("Adapter API error: chat.postMessage failed: ")
        );

        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(body.contains("chat.postMessage failed"), "body: {body}");
    }

    #[tokio::test]
    async fn provider_response_invalid_keeps_json_source() {
        let json_err = serde_json::from_str::<serde_json::Value>("{oops").unwrap_err();
        let err: AdapterError =
            SharedAdapterError::provider_response_invalid("Linear JSON", json_err).into();

        let chain = source_chain(&err);
        assert_eq!(chain.len(), 2, "decode error + serde error: {chain:?}");
        let source = err.source().expect("decode failure must stay reachable");
        assert!(
            source
                .source()
                .and_then(|s| s.downcast_ref::<serde_json::Error>())
                .is_some()
        );

        let (status, _) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    /// A provider that answered and said no is not a wrapped failure — it stays
    /// a synthesized message with no source, and callers can tell it apart from
    /// [`SharedAdapterError::ProviderUnreachable`].
    #[test]
    fn provider_rejection_is_synthesized_and_sourceless() {
        let err = SharedAdapterError::AdapterApi("chat.postMessage: channel_not_found".to_string());
        assert!(err.source().is_none());
        assert!(!matches!(
            err,
            SharedAdapterError::ProviderUnreachable { .. }
        ));
    }

    /// The two cross-band variants box their source, which is what takes the
    /// other band's error type out of this crate's dependency list — so what
    /// must keep working is reaching the cause via `Error::source` and
    /// downcasting it.
    #[tokio::test]
    async fn an_upstream_credential_failure_keeps_its_source_on_the_502_path() {
        let err = AdapterError::upstream_credential(
            "resolve the GitHub token for user u-1",
            sqlx::Error::PoolClosed,
        );

        assert!(
            err.source()
                .and_then(|s| s.downcast_ref::<sqlx::Error>())
                .is_some()
        );
        assert_eq!(
            err.to_string(),
            "Adapter API error: resolve the GitHub token for user u-1"
        );

        let (status, _) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    /// A message the adapter layer writes itself has nothing to carry.
    #[test]
    fn synthesized_internal_message_has_no_source() {
        assert!(
            SharedAdapterError::Internal("Slack adapter downcast failed".into())
                .source()
                .is_none()
        );
    }

    /// Every deterministic, sourceless variant renders a pinned status and the
    /// exact `{"error":...}` body a client sees. A later move of any variant
    /// must preserve both, so they are asserted literally, not by substring.
    #[tokio::test]
    async fn plain_variants_pin_status_body_and_are_sourceless() {
        let cases: Vec<(AdapterError, StatusCode, &str)> = vec![
            (
                SharedAdapterError::ConfigNotFound("connection".into()).into(),
                StatusCode::NOT_FOUND,
                r#"{"error":"Adapter config not found: connection"}"#,
            ),
            (
                SharedAdapterError::NotFound("task 7".into()).into(),
                StatusCode::NOT_FOUND,
                r#"{"error":"Not found: task 7"}"#,
            ),
            (
                SharedAdapterError::BadRequest("missing repository.full_name".into()).into(),
                StatusCode::BAD_REQUEST,
                r#"{"error":"Invalid request: missing repository.full_name"}"#,
            ),
            (
                SharedAdapterError::Conflict("already linked".into()).into(),
                StatusCode::CONFLICT,
                r#"{"error":"Conflict: already linked"}"#,
            ),
            (
                SharedAdapterError::AdapterNotConfigured("github_pr".into()).into(),
                StatusCode::BAD_REQUEST,
                r#"{"error":"Adapter not configured: github_pr"}"#,
            ),
            (
                SharedAdapterError::SignatureInvalid.into(),
                StatusCode::UNAUTHORIZED,
                r#"{"error":"Signature verification failed"}"#,
            ),
            (
                SharedAdapterError::AdapterApi("channel_not_found".into()).into(),
                StatusCode::BAD_GATEWAY,
                r#"{"error":"Adapter API error: channel_not_found"}"#,
            ),
            (
                SharedAdapterError::Unauthorized("token expired".into()).into(),
                StatusCode::BAD_GATEWAY,
                r#"{"error":"Adapter API error: token expired"}"#,
            ),
            (
                SharedAdapterError::AccessDenied("not a member".into()).into(),
                StatusCode::FORBIDDEN,
                r#"{"error":"Access denied: not a member"}"#,
            ),
            (
                SharedAdapterError::Overloaded.into(),
                StatusCode::SERVICE_UNAVAILABLE,
                r#"{"error":"Service temporarily unavailable"}"#,
            ),
        ];

        for (err, status, body) in cases {
            assert!(err.source().is_none(), "{err} must carry no source");
            let (got_status, got_body) = response_snapshot(err.into_response()).await;
            assert_eq!(got_status, status, "status for {got_body}");
            assert_eq!(got_body, body);
        }
    }

    /// The variants that wrap an internal cause all render one redacted 500 —
    /// the cause never reaches the client — while staying reachable through
    /// `source()` for the logs; `Internal` renders the same body with nothing
    /// to carry.
    #[tokio::test]
    async fn redacted_500_variants_hide_their_cause_but_keep_the_source() {
        const REDACTED: &str = r#"{"error":"Internal server error"}"#;

        let task_creation = AdapterError::task_handoff(
            "open a task from a provider event",
            sqlx::Error::RowNotFound,
        );
        assert!(
            task_creation
                .source()
                .and_then(|s| s.downcast_ref::<sqlx::Error>())
                .is_some()
        );
        let (status, body) = response_snapshot(task_creation.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, REDACTED);

        let message_delivery = AdapterError::task_handoff(
            "deliver a provider turn to a task",
            sqlx::Error::RowNotFound,
        );
        assert!(
            message_delivery
                .source()
                .and_then(|s| s.downcast_ref::<sqlx::Error>())
                .is_some()
        );
        let (status, body) = response_snapshot(message_delivery.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, REDACTED);

        let config_serialization: AdapterError = SharedAdapterError::ConfigSerialization(
            serde_json::from_str::<serde_json::Value>("{oops").unwrap_err(),
        )
        .into();
        assert!(
            config_serialization
                .source()
                .and_then(|s| s.downcast_ref::<serde_json::Error>())
                .is_some()
        );
        let (status, body) = response_snapshot(config_serialization.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, REDACTED);

        let internal: AdapterError =
            SharedAdapterError::Internal("Slack adapter downcast failed".into()).into();
        assert!(internal.source().is_none());
        let (status, body) = response_snapshot(internal.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, REDACTED);
    }

    /// The multi-target picker is the one variant with a bespoke body: a 400
    /// carrying the targets and the disambiguation hint, not the redacted
    /// envelope.
    #[tokio::test]
    async fn multiple_targets_pins_its_picker_body() {
        let err: AdapterError = SharedAdapterError::MultipleTargets {
            targets: vec![ReplyTarget {
                kind: "binding".to_string(),
                id: "binding-1".to_string(),
                label: "issue ENG-1".to_string(),
                flag_hint: "--binding binding-1".to_string(),
            }],
        }
        .into();
        assert!(err.source().is_none());
        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            r#"{"error":"Task has multiple reply targets; pick one explicitly","targets":[{"kind":"binding","id":"binding-1","label":"issue ENG-1","flagHint":"--binding binding-1"}],"hint":"Use --binding <binding_id> to pick one."}"#
        );
    }
}
