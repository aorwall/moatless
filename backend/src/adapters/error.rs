//! Adapter error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use moatless_api_schemas::bindings::{MultipleTargetsErrorBody, ReplyTargetDescriptor};

use crate::access_control::AccessControlError;
use crate::error::error_response;

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
pub enum AdapterError {
    #[error("Adapter config not found: {0}")]
    ConfigNotFound(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

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
    /// a body that died mid-read. Distinct from [`AdapterError::AdapterApi`] —
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

    /// Resolving the GitHub credential bound to a repository failed while
    /// serving an adapter API route. Stays on the provider-facing 502 path.
    #[error("Adapter API error: {source}")]
    RepositoryCredential {
        #[source]
        source: crate::repository::error::RepositoryError,
    },

    /// Resolving a user's GitHub token for an outbound adapter call failed.
    #[error("Adapter API error: Failed to resolve GitHub token for user {user_id}: {source}")]
    UserCredential {
        user_id: String,
        #[source]
        source: crate::auth::error::AuthError,
    },

    #[error("Adapter API error: {0}")]
    Unauthorized(String),

    #[error("Access denied: {0}")]
    AccessDenied(String),

    /// Returned by the no-flag send endpoint when a task has more than one
    /// reply target — the caller must pick one with `--thread`,
    /// `--binding`, or `--pr`.
    #[error("Task has multiple reply targets; pick one explicitly")]
    MultipleTargets { targets: Vec<ReplyTargetDescriptor> },

    /// Creating a task from a provider event failed for a transient reason.
    /// Preflight failures are handled at the call site and never reach here.
    #[error("Failed to create task from provider event")]
    TaskCreation(#[source] crate::task::creation::TaskCreationError),

    /// Handing an inbound provider message to the task message flow failed.
    #[error("Failed to deliver provider message to task")]
    MessageDelivery(#[source] crate::message::error::MessageError),

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

impl AdapterError {
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

impl From<AccessControlError> for AdapterError {
    fn from(value: AccessControlError) -> Self {
        match value {
            AccessControlError::BadRequest(msg) => Self::BadRequest(msg),
            AccessControlError::Forbidden(msg) => Self::AccessDenied(msg),
        }
    }
}

impl IntoResponse for AdapterError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AdapterError::ConfigNotFound(_) | AdapterError::NotFound(_) => {
                (StatusCode::NOT_FOUND, self.to_string())
            }
            AdapterError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AdapterError::AdapterNotConfigured(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AdapterError::SignatureInvalid => (StatusCode::UNAUTHORIZED, self.to_string()),
            AdapterError::Db(e) => {
                tracing::error!(error = %e, "Database error in adapter handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AdapterError::AccessDenied(_) => (StatusCode::FORBIDDEN, self.to_string()),
            AdapterError::AdapterApi(_)
            | AdapterError::Unauthorized(_)
            | AdapterError::ProviderUnreachable { .. }
            | AdapterError::ProviderResponseInvalid { .. }
            | AdapterError::RepositoryCredential { .. }
            | AdapterError::UserCredential { .. } => (StatusCode::BAD_GATEWAY, self.to_string()),
            AdapterError::MultipleTargets { targets } => {
                let body = MultipleTargetsErrorBody {
                    error: self.to_string(),
                    targets: targets.clone(),
                    hint: "Use --binding <binding_id> to pick one.".to_string(),
                };
                return (StatusCode::BAD_REQUEST, Json(body)).into_response();
            }
            AdapterError::TaskCreation(e) => {
                tracing::error!(error = %e, "Failed to create task from provider event");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AdapterError::MessageDelivery(e) => {
                tracing::error!(error = %e, "Failed to deliver provider message to task");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AdapterError::ConfigSerialization(e) => {
                tracing::error!(error = %e, "Failed to serialize adapter config");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AdapterError::Overloaded => {
                tracing::warn!("Adapter delivery admission refused: backlog at capacity");
                (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
            }
            AdapterError::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal error in adapter handler");
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
    use std::error::Error as _;

    use super::*;
    use crate::adapters::test_support::{response_snapshot, source_chain, transport_error};

    /// `Db` already carried its cause; the lookup sites that used to stringify
    /// a `sqlx::Error` into `Internal` now route through it.
    #[tokio::test]
    async fn db_lookup_keeps_sqlx_source_and_stays_redacted() {
        let err = AdapterError::Db(sqlx::Error::RowNotFound);

        assert!(err
            .source()
            .and_then(|s| s.downcast_ref::<sqlx::Error>())
            .is_some());

        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
    }

    #[tokio::test]
    async fn provider_unreachable_keeps_reqwest_source() {
        let err = AdapterError::provider_unreachable("chat.postMessage", transport_error().await);

        let source = err.source().expect("transport failure must stay reachable");
        assert!(source.downcast_ref::<reqwest::Error>().is_some());
        assert!(err
            .to_string()
            .starts_with("Adapter API error: chat.postMessage failed: "));

        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(body.contains("chat.postMessage failed"), "body: {body}");
    }

    #[tokio::test]
    async fn provider_response_invalid_keeps_json_source() {
        let json_err = serde_json::from_str::<serde_json::Value>("{oops").unwrap_err();
        let err = AdapterError::provider_response_invalid("Linear JSON", json_err);

        let chain = source_chain(&err);
        assert_eq!(chain.len(), 2, "decode error + serde error: {chain:?}");
        let source = err.source().expect("decode failure must stay reachable");
        assert!(source
            .source()
            .and_then(|s| s.downcast_ref::<serde_json::Error>())
            .is_some());

        let (status, _) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    /// A provider that answered and said no is not a wrapped failure — it stays
    /// a synthesized message with no source, and callers can tell it apart from
    /// [`AdapterError::ProviderUnreachable`].
    #[test]
    fn provider_rejection_is_synthesized_and_sourceless() {
        let err = AdapterError::AdapterApi("chat.postMessage: channel_not_found".to_string());
        assert!(err.source().is_none());
        assert!(!matches!(err, AdapterError::ProviderUnreachable { .. }));
    }

    #[tokio::test]
    async fn repository_credential_keeps_source_on_the_502_path() {
        let err = AdapterError::RepositoryCredential {
            source: crate::repository::error::RepositoryError::Db(sqlx::Error::PoolClosed),
        };

        assert!(err
            .source()
            .and_then(|s| s.downcast_ref::<crate::repository::error::RepositoryError>())
            .is_some());

        let (status, _) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn user_credential_keeps_auth_source_and_message() {
        let err = AdapterError::UserCredential {
            user_id: "u-1".to_string(),
            source: crate::auth::error::AuthError::Forbidden,
        };

        assert!(err
            .source()
            .and_then(|s| s.downcast_ref::<crate::auth::error::AuthError>())
            .is_some());
        assert_eq!(
            err.to_string(),
            "Adapter API error: Failed to resolve GitHub token for user u-1: Access denied"
        );

        let (status, _) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    /// A message the adapter layer writes itself has nothing to carry.
    #[test]
    fn synthesized_internal_message_has_no_source() {
        assert!(
            AdapterError::Internal("Slack adapter downcast failed".into())
                .source()
                .is_none()
        );
    }
}
