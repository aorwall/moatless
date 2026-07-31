use std::ffi::OsString;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use tokio::sync::{Mutex, MutexGuard, oneshot};
use tokio::task::JoinHandle;

static GITHUB_API_ENV_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

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
    requests: Arc<Mutex<Vec<RecordedGithubRequest>>>,
    previous_base_url: Option<OsString>,
    shutdown: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
    _env_lock: MutexGuard<'static, ()>,
}

impl GithubApiStub {
    pub(super) async fn recorded_requests(&self) -> Vec<RecordedGithubRequest> {
        self.requests.lock().await.clone()
    }
}

impl Drop for GithubApiStub {
    fn drop(&mut self) {
        if let Some(value) = self.previous_base_url.take() {
            std::env::set_var("GITHUB_API_BASE_URL", value);
        } else {
            std::env::remove_var("GITHUB_API_BASE_URL");
        }
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.handle.abort();
    }
}

#[derive(Clone)]
struct StubState {
    requests: Arc<Mutex<Vec<RecordedGithubRequest>>>,
    responder: Arc<dyn Fn(Method, &str, &str) -> StubResponse + Send + Sync>,
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

    let env_lock = GITHUB_API_ENV_LOCK.lock().await;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let previous_base_url = std::env::var_os("GITHUB_API_BASE_URL");
    std::env::set_var("GITHUB_API_BASE_URL", format!("http://{addr}"));

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
        requests,
        previous_base_url,
        shutdown: Some(shutdown),
        handle,
        _env_lock: env_lock,
    }
}
