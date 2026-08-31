//! Test-only helpers that travel with the Adapter contract.
//!
//! Two groups, both about something this crate defines. The error helpers read
//! a rendered response back into `(status, body)`, walk a source chain, and
//! produce a genuine [`reqwest::Error`] from a refused connection; they live
//! beside [`crate::error`] because they exist to assert it, and a copy parked
//! in a band above would drift from the renderer it is checking.
//!
//! The loopback stubs give a provider client somewhere local to talk to: an
//! address nothing listens on ([`UNREACHABLE_BASE`], transport failure), a
//! server that answers with a body of our choosing ([`stub_provider`],
//! rejection or undecodable payload), and a recorder that distinguishes
//! endpoints and so makes a workflow'"'"'s API-call budget assertable
//! ([`ProviderStub`]). Adapter error classification only means something if the
//! real request path produces it, which is what these are for.

// Compiled for two consumers — in-crate `#[cfg(test)]` modules and the crates
// that enable `test-support` — and neither uses all of it.
#![allow(dead_code, reason = "shared across two test tiers; each uses a subset")]
#![allow(
    clippy::expect_used,
    reason = "a broken test fixture should fail the test loudly, not be handled"
)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};

use moatless_types::store::StoreError;

use crate::delivery::{AdapterDeliveryRequest, Admission, AdmitError, DeliveryAdmission};

/// What a [`FakeAdmission`] answers. A descriptor rather than a stored
/// `Result`, because [`AdmitError`] is not `Clone` and a fake that answered
/// once could not pin a retry.
#[derive(Debug, Clone, Copy)]
pub enum FakeAdmissionAnswer {
    Admitted,
    Duplicate,
    AtCapacity { limit: i64 },
    StoreFailed,
}

/// A [`DeliveryAdmission`] that answers a fixed outcome and records every
/// request, so a handler test can drive each admission branch without a
/// database — the outcomes the concrete store reaches only against real
/// Postgres state.
pub struct FakeAdmission {
    answer: FakeAdmissionAnswer,
    seen: Mutex<Vec<AdapterDeliveryRequest>>,
}

impl FakeAdmission {
    pub fn new(answer: FakeAdmissionAnswer) -> Self {
        Self {
            answer,
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Every request handed to [`DeliveryAdmission::admit`], in order. What a
    /// test asserts the `kind` and the sender's `event_id` on.
    pub fn seen(&self) -> Vec<AdapterDeliveryRequest> {
        self.seen.lock().expect("seen log is not poisoned").clone()
    }
}

#[async_trait]
impl DeliveryAdmission for FakeAdmission {
    async fn admit(&self, request: AdapterDeliveryRequest) -> Result<Admission, AdmitError> {
        self.seen
            .lock()
            .expect("seen log is not poisoned")
            .push(request);
        match self.answer {
            FakeAdmissionAnswer::Admitted => Ok(Admission::Admitted),
            FakeAdmissionAnswer::Duplicate => Ok(Admission::Duplicate),
            FakeAdmissionAnswer::AtCapacity { limit } => Err(AdmitError::AtCapacity { limit }),
            FakeAdmissionAnswer::StoreFailed => Err(AdmitError::Store(StoreError::new(
                "persist the Adapter delivery record",
            ))),
        }
    }
}

/// An address on loopback nothing is listening on, so connecting is refused
/// immediately. Injected as a provider client'"'"'s base URL to exercise the "we
/// could not reach the provider" path deterministically and offline.
pub const UNREACHABLE_BASE: &str = "http://127.0.0.1:1";

/// A genuine [`reqwest::Error`] from a refused connection.
pub async fn transport_error() -> reqwest::Error {
    reqwest::Client::new()
        .get(UNREACHABLE_BASE)
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

/// Serve `status` + `body` on a loopback port for every path, and return the
/// base URL. The server lives as long as the test process.
pub async fn stub_provider(status: StatusCode, body: &'static str) -> String {
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
pub struct RecordedRequest {
    pub method: String,
    /// Path only, e.g. `/users.info`.
    pub path: String,
    /// Raw query string, empty when the request carried none.
    pub query: String,
    /// Request body as text, empty for bodyless requests.
    pub body: String,
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
pub struct ProviderStub {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl ProviderStub {
    /// Serve `responder(path)` as a JSON body on every route. The server lives
    /// as long as the test process.
    pub async fn start<F>(responder: F) -> Self
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

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .expect("stub request log is not poisoned")
            .clone()
    }

    /// The paths hit, in order — the API-call budget of whatever just ran.
    pub fn paths(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| request.path)
            .collect()
    }

    /// The GraphQL operations invoked, in order.
    ///
    /// [`Self::paths`] cannot express the budget of a GraphQL sender: every
    /// call is `POST /graphql`, so a workflow that made three round trips and
    /// one that made one look identical. The operation is the only thing that
    /// distinguishes them, and it is in the body — the top-level field of the
    /// `query` document, which is `issue` for a read and the mutation name for
    /// a write. Requests that carry no recognisable document are reported by
    /// their path rather than dropped — an OAuth token refresh is a real round
    /// trip, and a budget that hid it would understate what the workflow cost.
    pub fn graphql_operations(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| {
                serde_json::from_str::<serde_json::Value>(&request.body)
                    .ok()
                    .as_ref()
                    .and_then(|body| body.get("query"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(graphql_operation_name)
                    .unwrap_or_else(|| format!("{} {}", request.method, request.path))
            })
            .collect()
    }
}

/// The top-level field of a GraphQL document — `issueUpdate` in
/// `mutation IssueUpdate($id: String!) { issueUpdate(...) { ... } }`, `issue`
/// in `query ($id: String!) { issue(id: $id) { ... } }`.
///
/// Deliberately a scan rather than a parse: the documents this reads are the
/// literal strings in `agent_api.rs`, and a budget assertion that failed
/// because a query gained a whitespace would be worse than useless.
fn graphql_operation_name(document: &str) -> Option<String> {
    let body = document.split_once('{')?.1;
    let field = body
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .find(|token| !token.is_empty())?;
    Some(field.to_string())
}
