//! `#[cfg(test)]` helpers for driving adapter HTTP clients without the network.
//!
//! Adapter error classification only means something if the real request path
//! produces it, so these helpers give the clients somewhere local to talk to:
//! an address nothing listens on (transport failure) or a stub that answers
//! with a body of our choosing (rejection / undecodable payload).

use axum::http::StatusCode;
use axum::routing::any;
use axum::Router;

/// An address on loopback that nothing is listening on. Connecting is refused
/// immediately, so this exercises the "we could not reach the provider" path
/// deterministically and offline.
pub(crate) const UNREACHABLE_BASE: &str = "http://127.0.0.1:1";

/// Serve `status` + `body` on a loopback port for every path, and return the
/// base URL. The server lives as long as the test process.
pub(crate) async fn stub_provider(status: StatusCode, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let app = Router::new().fallback(any(move || async move { (status, body) }));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

/// A genuine [`reqwest::Error`] from a refused connection.
pub(crate) async fn transport_error() -> reqwest::Error {
    reqwest::Client::new()
        .get(UNREACHABLE_BASE)
        .send()
        .await
        .expect_err("connection to a dead loopback port must fail")
}

/// Walk `Error::source` and collect each link's `Display`.
pub(crate) fn source_chain(err: &dyn std::error::Error) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current = err.source();
    while let Some(e) = current {
        chain.push(e.to_string());
        current = e.source();
    }
    chain
}

/// Read an `IntoResponse` result into `(status, body)` for response snapshots.
pub(crate) async fn response_snapshot(response: axum::response::Response) -> (StatusCode, String) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}
