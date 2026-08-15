//! Shared adapter error vocabulary.
//!
//! [`SharedAdapterError`] is the band-local half of the adapter error space:
//! the variants that name nothing above the adapter-model band, carrying only
//! `String`, unit, `sqlx::Error`, `reqwest::Error`, `serde_json::Error`, or
//! `Vec<ReplyTarget>`. It has no `IntoResponse`; the HTTP renderer
//! lives with the backend `AdapterError` that wraps this and adds the
//! up-naming credential/task/message variants.

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
/// buffered body (`serde_json::from_str`); both causes stay reachable through
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

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    /// The provider answered and rejected the call — a non-success status, or
    /// an `ok: false` / `errors` payload. Synthesized from the provider's own
    /// words, so there is no underlying Rust error to carry.
    #[error("Adapter API error: {0}")]
    AdapterApi(String),

    /// We never got an answer from the provider: DNS, TLS, connect, timeout, or
    /// a body that died mid-read. Distinct from [`SharedAdapterError::AdapterApi`] —
    /// callers that must tell "provider rejected us" from "we could not reach
    /// the provider" match on this.
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
    /// reply target — the caller must pick one with `--thread`,
    /// `--binding`, or `--pr`.
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
    /// The provider call never completed — keep the transport failure as source.
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
