//! Test-only helpers that travel with the Adapter error contract.
//!
//! Three things the error tests need and nothing else in the kernel does:
//! reading a rendered response back into `(status, body)`, walking a source
//! chain, and producing a genuine [`reqwest::Error`] from a refused connection.
//! They live beside [`crate::error`] because they exist to assert it — a copy
//! parked in a band above would drift from the renderer it is checking.
//!
//! The loopback *stubs* (a server that answers with a chosen body, a recorder
//! that pins a workflow's API-call budget) stay with the provider clients they
//! drive; they are about a provider's HTTP surface, not about this error type.

// Compiled for two consumers — in-crate `#[cfg(test)]` modules and the crates
// that enable `test-support` — and neither uses all of it.
#![allow(dead_code, reason = "shared across two test tiers; each uses a subset")]
#![allow(
    clippy::expect_used,
    reason = "a broken test fixture should fail the test loudly, not be handled"
)]

use axum::http::StatusCode;

/// An address on loopback nothing is listening on, so connecting is refused
/// immediately. Private: callers want "a transport failure", not this address.
/// `moatless_loops::adapters::test_support::UNREACHABLE_BASE` is the public
/// affordance for injecting an unreachable base URL into a provider client —
/// a different job that happens to want the same kind of address.
const UNREACHABLE: &str = "http://127.0.0.1:1";

/// A genuine [`reqwest::Error`] from a refused connection.
pub async fn transport_error() -> reqwest::Error {
    reqwest::Client::new()
        .get(UNREACHABLE)
        .send()
        .await
        .expect_err("connection to a dead loopback port must fail")
}

/// Walk `Error::source` and collect each link's `Display`.
pub fn source_chain(err: &dyn std::error::Error) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current = err.source();
    while let Some(e) = current {
        chain.push(e.to_string());
        current = e.source();
    }
    chain
}

/// Read an `IntoResponse` result into `(status, body)` for response snapshots.
pub async fn response_snapshot(response: axum::response::Response) -> (StatusCode, String) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}
