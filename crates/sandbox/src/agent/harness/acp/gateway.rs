//! A loopback proxy from an ACP agent to the backend's model gateway.
//!
//! It presents the task token current at each request: agy reads its key once,
//! at spawn, while the backend rotates the token with each new driver, so a
//! key passed at spawn would force a respawn and refuse mid-turn steering. It
//! also gives agy, which accepts plain HTTP only on loopback, a loopback URL.
//! Last, it restores a model suffix agy strips: see [`GatewayProxy::select`].

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use moatless_types::http::HttpStreamTimeouts;
use tokio::task::JoinHandle;

use super::agent::task_token;
use crate::config::Config;
use crate::credentials::github_proxy::forwards;

/// The key an agent is spawned with; the proxy replaces it with the task token.
pub(super) const PLACEHOLDER_KEY: &str = "moatless-gateway-proxy";

#[derive(Clone)]
struct ProxyState {
    config: Arc<Config>,
    /// `BACKEND_API_URL` without a trailing `/`.
    backend: String,
    /// The path every forwarded URL must start with: the backend's `/llm/`.
    llm_prefix: String,
    http: reqwest::Client,
    selected: Arc<Mutex<Option<String>>>,
}

/// Suffixes `agy` turns into a thinking level and drops from a Gemini id.
const THINKING_SUFFIXES: &[&str] = &["-high", "-xhigh"];

/// The largest request body buffered to restore a suffix.
const MAX_REWRITTEN_BODY: usize = 64 * 1024 * 1024;

/// Serves on a loopback port until dropped.
pub(super) struct GatewayProxy {
    port: u16,
    serve: JoinHandle<()>,
    selected: Arc<Mutex<Option<String>>>,
}

impl GatewayProxy {
    /// Binds `127.0.0.1:0` and forwards `/llm/…` to `backend`.
    pub(super) async fn start(config: Arc<Config>, backend: &str) -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let backend = backend.trim_end_matches('/').to_string();
        let base_path = reqwest::Url::parse(&backend)
            .map_err(std::io::Error::other)?
            .path()
            .trim_end_matches('/')
            .to_string();
        let selected = Arc::default();
        let state = ProxyState {
            http: client(config.http.model_gateway)?,
            config,
            backend,
            llm_prefix: format!("{base_path}/llm/"),
            selected: Arc::clone(&selected),
        };
        let app = Router::new().fallback(forward).with_state(state);
        let serve = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                tracing::warn!(%error, "ACP gateway proxy stopped");
            }
        });
        Ok(Self {
            port,
            serve,
            selected,
        })
    }

    /// Records the model the session runs, so a request naming it without
    /// its thinking suffix is sent as `model`.
    ///
    /// `agy` sends `gemini-4-argon-high` as `gemini-4-argon`, an id the hub
    /// does not serve, while it keeps the suffix on other vendors' ids.
    pub(super) fn select(&self, model: Option<&str>) {
        let stripped = model.filter(|m| THINKING_SUFFIXES.iter().any(|s| m.ends_with(s)));
        *self
            .selected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = stripped.map(str::to_string);
    }

    /// The base URL the agent appends `/v1/chat/completions` to.
    pub(super) fn url(&self) -> String {
        format!("http://127.0.0.1:{}/llm/openai", self.port)
    }
}

impl Drop for GatewayProxy {
    fn drop(&mut self) {
        self.serve.abort();
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "sets the timeouts `Config::http` holds"
)]
fn client(timeouts: HttpStreamTimeouts) -> std::io::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .read_timeout(timeouts.idle)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(std::io::Error::other)
}

async fn forward(State(state): State<ProxyState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let path_and_query = parts.uri.path_and_query().map_or("/", |p| p.as_str());
    // Checked after parsing: the parser resolves `..` and `%2e%2e`, so a raw
    // `/llm/../api/…` would otherwise reach other routes with the task token.
    let url = reqwest::Url::parse(&format!("{}{path_and_query}", state.backend));
    let Some(url) = url.ok().filter(|u| u.path().starts_with(&state.llm_prefix)) else {
        return error(StatusCode::NOT_FOUND, "Only the model gateway is proxied");
    };
    let Some(token) = task_token(&state.config).await else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "This sandbox holds no task token yet",
        );
    };
    let selected = state
        .selected
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let (body, rewritten) = match selected {
        Some(model) => match axum::body::to_bytes(body, MAX_REWRITTEN_BODY).await {
            Ok(bytes) => {
                let restored = restore_suffix(&bytes, &model);
                let rewritten = restored.is_some();
                (reqwest::Body::from(restored.unwrap_or(bytes)), rewritten)
            }
            Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "The request is too large"),
        },
        None => (
            reqwest::Body::wrap_stream(body.into_data_stream().map_err(std::io::Error::other)),
            false,
        ),
    };
    let mut upstream = state.http.request(parts.method, url).body(body);
    for (name, value) in &parts.headers {
        // The agent's own `Authorization` carries `PLACEHOLDER_KEY`.
        if forwards(name)
            && name != header::AUTHORIZATION
            && !(rewritten && name == header::CONTENT_LENGTH)
        {
            upstream = upstream.header(name, value);
        }
    }
    let response = match upstream.bearer_auth(token).send().await {
        Ok(response) => response,
        Err(e) => {
            tracing::warn!(error = %e, "An ACP model call did not reach the backend");
            return error(StatusCode::BAD_GATEWAY, "The backend is unreachable");
        }
    };
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut out = Response::builder().status(status);
    for (name, value) in response.headers() {
        // `reqwest` already dechunked the body.
        if forwards(name) && name != header::CONTENT_LENGTH {
            if let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) {
                out = out.header(name.clone(), value);
            }
        }
    }
    out.body(Body::from_stream(response.bytes_stream()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// `body` with its `model` set to `selected`, when it names `selected` minus
/// its thinking suffix; `None` leaves the body as sent.
fn restore_suffix(body: &[u8], selected: &str) -> Option<Bytes> {
    let mut json: serde_json::Value = serde_json::from_slice(body).ok()?;
    let sent = json.get("model")?.as_str()?;
    let stem_matches = THINKING_SUFFIXES
        .iter()
        .any(|suffix| selected.strip_suffix(suffix) == Some(sent));
    if !stem_matches {
        return None;
    }
    json["model"] = selected.into();
    serde_json::to_vec(&json).ok().map(Bytes::from)
}

fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({ "message": message })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::http::HeaderMap;

    use super::*;

    /// Each request's path and `Authorization`, in arrival order.
    type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

    /// Sends `path` unnormalized, as a client that skips URL parsing would.
    async fn raw_status(port: u16, path: &str) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let request = format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).await.unwrap();
        response[9..12].parse().unwrap()
    }

    #[tokio::test]
    async fn each_request_carries_the_token_current_at_that_moment() {
        let seen: Seen = Arc::default();
        let recorded = Arc::clone(&seen);
        let backend = Router::new().fallback(move |uri: axum::http::Uri, headers: HeaderMap| {
            let recorded = Arc::clone(&recorded);
            async move {
                let auth = headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                recorded.lock().unwrap().push((uri.to_string(), auth));
                "ok"
            }
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, backend).await.unwrap() });

        let dir = tempfile::TempDir::new().unwrap();
        let mut config = crate::config::test_support::minimal_config();
        config.runtime_dir = dir.path().display().to_string();
        let config = Arc::new(config);
        let token_path = config.persisted_user_token_path();
        let proxy = GatewayProxy::start(Arc::clone(&config), &format!("http://{addr}/prefix/"))
            .await
            .unwrap();
        let http = client(config.http.model_gateway).unwrap();
        let call = |path: &str| {
            http.post(format!("{}{path}", proxy.url()))
                .bearer_auth(PLACEHOLDER_KEY)
                .send()
        };

        tokio::fs::write(&token_path, "first").await.unwrap();
        assert_eq!(call("/v1/chat/completions").await.unwrap().status(), 200);
        tokio::fs::write(&token_path, "second").await.unwrap();
        assert_eq!(call("/v1/models").await.unwrap().status(), 200);
        for path in [
            "/api/v1/tasks",
            "/llm/../api/v1/tasks",
            "/llm/%2e%2e/api/v1/tasks",
            "/llm/openai/%2E%2E/%2e%2e/api/v1/tasks",
        ] {
            assert_eq!(raw_status(proxy.port, path).await, 404, "{path}");
        }

        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                (
                    "/prefix/llm/openai/v1/chat/completions".to_string(),
                    Some("Bearer first".to_string())
                ),
                (
                    "/prefix/llm/openai/v1/models".to_string(),
                    Some("Bearer second".to_string())
                ),
            ]
        );
    }

    #[test]
    fn a_model_sent_without_the_selected_thinking_suffix_gets_it_back() {
        let model = |body: Option<Bytes>| {
            body.map(|b| serde_json::from_slice::<serde_json::Value>(&b).unwrap()["model"].clone())
        };
        let body =
            |m: &str| serde_json::to_vec(&serde_json::json!({"model": m, "stream": true})).unwrap();
        for (selected, sent, expected) in [
            (
                "gemini-4-argon-high",
                "gemini-4-argon",
                Some("gemini-4-argon-high"),
            ),
            (
                "gemini-4-neon-xhigh",
                "gemini-4-neon",
                Some("gemini-4-neon-xhigh"),
            ),
            ("gemini-4-argon-high", "gemini-4-argon-high", None),
            // agy's own utility calls name another model.
            ("gemini-4-argon-high", "gemini-3.1-flash-lite", None),
        ] {
            assert_eq!(
                model(restore_suffix(&body(sent), selected)),
                expected.map(serde_json::Value::from),
                "{selected} / {sent}"
            );
        }
        assert_eq!(restore_suffix(b"not json", "gemini-4-argon-high"), None);
    }
}
