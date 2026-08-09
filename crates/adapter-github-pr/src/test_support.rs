//! A local GitHub REST stub that records what the Adapter sent it.
//!
//! The stub's address reaches the Adapter through
//! [`super::adapter::GithubPrAdapter::with_api_base`], not through the
//! `GITHUB_API_BASE_URL` process-global this used to write: a global is shared
//! with every other test in the binary, so the stub both saw requests it never
//! provoked and serialized every GitHub stub test behind one mutex.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordedGithubRequest {
    pub(super) method: String,
    pub(super) uri: String,
    pub(super) authorization: Option<String>,
}

pub(super) enum StubResponse {
    Json(StatusCode, serde_json::Value),
    Empty(StatusCode),
}

impl StubResponse {
    pub(super) fn json(status: StatusCode, body: serde_json::Value) -> Self {
        Self::Json(status, body)
    }

    pub(super) fn empty(status: StatusCode) -> Self {
        Self::Empty(status)
    }

    pub(super) fn not_found(uri: &str) -> Self {
        Self::json(
            StatusCode::NOT_FOUND,
            serde_json::json!({"message": format!("unexpected {uri}")}),
        )
    }
}

impl IntoResponse for StubResponse {
    fn into_response(self) -> Response {
        match self {
            Self::Json(status, body) => (status, Json(body)).into_response(),
            Self::Empty(status) => status.into_response(),
        }
    }
}

pub(super) struct GithubApiStub {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedGithubRequest>>>,
    shutdown: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
}

impl GithubApiStub {
    /// Hand this to [`super::adapter::GithubPrAdapter::with_api_base`].
    pub(super) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(super) async fn recorded_requests(&self) -> Vec<RecordedGithubRequest> {
        self.requests.lock().await.clone()
    }
}

impl Drop for GithubApiStub {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.handle.abort();
    }
}

/// Shared responder closure — aliased because the bare `Arc<dyn Fn(..)>` trips
/// the workspace-denied `clippy::type_complexity`.
type StubResponder = Arc<dyn Fn(Method, &str, &str) -> StubResponse + Send + Sync>;

#[derive(Clone)]
struct StubState {
    requests: Arc<Mutex<Vec<RecordedGithubRequest>>>,
    responder: StubResponder,
}

pub(super) async fn start_github_api_stub<F>(responder: F) -> GithubApiStub
where
    F: Fn(Method, &str, &str) -> StubResponse + Send + Sync + 'static,
{
    async fn handle(State(state): State<StubState>, request: Request) -> Response {
        let method = request.method().clone();
        let path = request.uri().path().to_string();
        let uri = request
            .uri()
            .path_and_query()
            .map_or_else(|| path.clone(), ToString::to_string);
        let authorization = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        state.requests.lock().await.push(RecordedGithubRequest {
            method: method.to_string(),
            uri: uri.clone(),
            authorization,
        });

        (state.responder)(method, &path, &uri).into_response()
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let state = StubState {
        requests: requests.clone(),
        responder: Arc::new(responder),
    };
    let app = Router::new().fallback(any(handle)).with_state(state);
    let (shutdown, shutdown_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });
        if let Err(error) = server.await {
            tracing::error!(%error, "GitHub API test stub server failed");
        }
    });

    GithubApiStub {
        base_url: format!("http://{addr}"),
        requests,
        shutdown: Some(shutdown),
        handle,
    }
}
