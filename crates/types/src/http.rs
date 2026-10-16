//! The standard JSON error body, the timeouts an HTTP client is built with,
//! and the marker for a relayed upstream failure.

use serde::Serialize;

/// Standard JSON error body. All API errors MUST use this shape.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ErrorBody {
    pub error: String,
}

/// Connect and whole-exchange bounds for one integration's HTTP client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpTimeouts {
    /// How long opening the connection, TLS included, may take.
    pub connect: std::time::Duration,
    /// How long one request may take, from sending it to reading the last byte.
    pub total: std::time::Duration,
}

/// Bounds for a client whose response body streams for as long as it moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpStreamTimeouts {
    /// How long opening the connection, TLS included, may take.
    pub connect: std::time::Duration,
    /// The longest silence allowed between two reads of the body.
    pub idle: std::time::Duration,
}

/// Marks a 5xx response that relays an upstream service's failure rather than
/// this backend's own, so request tracing logs it at WARN instead of ERROR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayedUpstreamFailure;
