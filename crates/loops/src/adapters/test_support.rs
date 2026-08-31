//! Test-only helpers for driving adapter HTTP clients without the network.
//!
//! Adapter error classification only means something if the real request path
//! produces it, so these helpers give the clients somewhere local to talk to:
//! an address nothing listens on (transport failure) or a stub that answers
//! with a body of our choosing (rejection / undecodable payload).
//!
//! The helpers that read a *rendered* `AdapterError` back are in
//! `moatless_adapter_model::test_support`, next to the renderer they check.

// Compiled for two consumers — in-crate `#[cfg(test)]` modules and the
// `db-tests` integration binaries — and neither uses all of it. Under the
// `db-tests` feature this module is part of the library rather than the test
// binary, so the library's lints apply to it and have to be waived here.
#![allow(dead_code, reason = "shared across two test tiers; each uses a subset")]
#![allow(
    clippy::expect_used,
    reason = "a broken test fixture should fail the test loudly, not be handled"
)]

use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};

/// An address on loopback that nothing is listening on. Connecting is refused
/// immediately, so this exercises the "we could not reach the provider" path
/// deterministically and offline.
pub const UNREACHABLE_BASE: &str = "http://127.0.0.1:1";

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

// `transport_error`, `source_chain` and `response_snapshot` were here. They
// assert the Adapter error contract, so they moved with it into
// `moatless_adapter_model::test_support`; name them from there. What is left in
// this module is about a *provider's* HTTP surface — a stub to talk to, a
// recorder for its call budget — which is the loops band's business.

/// Thin pass-throughs over the registry's capability accessors.
///
/// The capability-matrix tests build the five concrete providers, which live in
/// the backend crate above the loops band, so those tests live there too. They
/// reach the kernel registry's lookups through this shared, test-context path
/// so the moved test asserts the exact production lookup and a second copy of
/// the call cannot drift from it. Gated with the rest of this module, so nothing
/// here reaches a release build.
pub mod registry {
    use moatless_adapter_model::capabilities::{
        AdapterBindingReply, AdapterMessageReply, AdapterReactions,
    };
    use moatless_adapter_model::error::AdapterError;
    use moatless_adapter_model::registry::Adapters;
    use moatless_types::adapter::AdapterKind;

    pub fn reactions(
        adapters: &Adapters,
        kind: AdapterKind,
    ) -> Result<&dyn AdapterReactions, AdapterError> {
        adapters.reactions(kind)
    }

    pub fn reactions_by_name<'a>(
        adapters: &'a Adapters,
        name: &str,
    ) -> Result<&'a dyn AdapterReactions, AdapterError> {
        adapters.reactions_by_name(name)
    }

    pub fn outbound_for_binding_reply_by_name<'a>(
        adapters: &'a Adapters,
        name: &str,
    ) -> Result<&'a dyn AdapterBindingReply, AdapterError> {
        adapters.outbound_for_binding_reply_by_name(name)
    }

    pub fn outbound_for_message_reply_by_name<'a>(
        adapters: &'a Adapters,
        name: &str,
    ) -> Result<&'a dyn AdapterMessageReply, AdapterError> {
        adapters.outbound_for_message_reply_by_name(name)
    }
}

/// Test-context inspection of the kinds registered on the durable-delivery
/// seam. The production collection stays private; installation contract tests
/// need only its observable claim set.
pub mod delivery {
    use super::super::delivery::AdapterDeliveryHandlers;

    pub fn kinds(handlers: &AdapterDeliveryHandlers) -> Vec<&'static str> {
        handlers.kinds()
    }
}
