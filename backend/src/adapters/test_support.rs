//! `#[cfg(test)]` helpers for driving adapter HTTP clients without the network.
//!
//! Adapter error classification only means something if the real request path
//! produces it, so these helpers give the clients somewhere local to talk to:
//! an address nothing listens on (transport failure) or a stub that answers
//! with a body of our choosing (rejection / undecodable payload).

use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};

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

/// One request a [`ProviderStub`] answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedRequest {
    pub(crate) method: String,
    /// Path only, e.g. `/users.info`.
    pub(crate) path: String,
    /// Raw query string, empty when the request carried none.
    pub(crate) query: String,
    /// Request body as text, empty for bodyless requests.
    pub(crate) body: String,
}

/// The per-path responder, aliased so the `Arc<dyn Fn(..)>` field stays under
/// `clippy::type_complexity`.
type StubResponder = Arc<dyn Fn(&str) -> serde_json::Value + Send + Sync>;

#[derive(Clone)]
struct StubState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    responder: StubResponder,
}

/// A loopback stand-in for a provider's HTTP API that answers each path from a
/// closure and records every request it saw.
///
/// Unlike [`stub_provider`] it distinguishes endpoints, which is what makes the
/// per-workflow API-call budgets assertable: the recorded paths *are* the calls
/// the code under test made.
pub(crate) struct ProviderStub {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl ProviderStub {
    /// Serve `responder(path)` as a JSON body on every route. The server lives
    /// as long as the test process.
    pub(crate) async fn start<F>(responder: F) -> Self
    where
        F: Fn(&str) -> serde_json::Value + Send + Sync + 'static,
    {
        async fn handle(State(state): State<StubState>, request: Request) -> Response {
            let method = request.method().to_string();
            let path = request.uri().path().to_string();
            let query = request.uri().query().unwrap_or_default().to_string();
            let (_, body) = request.into_parts();
            let body = axum::body::to_bytes(body, usize::MAX)
                .await
                .unwrap_or_default();
            let response = (state.responder)(&path);
            state
                .requests
                .lock()
                .expect("stub request log is not poisoned")
                .push(RecordedRequest {
                    method,
                    path,
                    query,
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
            Json(response).into_response()
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = StubState {
            requests: requests.clone(),
            responder: Arc::new(responder),
        };
        let app = Router::new().fallback(any(handle)).with_state(state);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            base_url: format!("http://{addr}"),
            requests,
        }
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .expect("stub request log is not poisoned")
            .clone()
    }

    /// The paths hit, in order — the API-call budget of whatever just ran.
    pub(crate) fn paths(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| request.path)
            .collect()
    }
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
