//! A loopback proxy from an ACP agent to the backend's model gateway.
//!
//! It presents the task token current at each request: agy reads its key once,
//! at spawn, while the backend rotates the token with each new driver, so a
//! key passed at spawn would force a respawn and refuse mid-turn steering. It
//! also gives agy, which accepts plain HTTP only on loopback, a loopback URL.
//! Last, it pins each request to the session's model and effort, and lists
//! only the account's models: see [`GatewayProxy::select`].

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
    selected: Arc<Mutex<Selection>>,
}

/// The model and effort the session runs.
#[derive(Clone, Default)]
struct Selection {
    model: Option<String>,
    effort: Option<String>,
}

/// The largest body buffered to rewrite it.
const MAX_REWRITTEN_BODY: usize = 64 * 1024 * 1024;

/// Serves on a loopback port until dropped.
pub(super) struct GatewayProxy {
    port: u16,
    serve: JoinHandle<()>,
    selected: Arc<Mutex<Selection>>,
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

    /// Records the model and effort the session runs; a request naming `model`,
    /// with or without its effort suffix, is sent as `model` at `effort`.
    ///
    /// `agy` sends `gemini-4-argon-high` as `gemini-4-argon`, which the hub
    /// does not serve, and takes the effort from the id alone.
    pub(super) fn select(&self, model: Option<&str>, effort: Option<&str>) {
        *self
            .selected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Selection {
            model: model.map(str::to_string),
            effort: effort.map(str::to_string),
        };
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
    let lists_models = parts.method == axum::http::Method::GET && url.path().ends_with("/models");
    let (body, rewritten) = if parts.method == axum::http::Method::POST && selected.model.is_some()
    {
        match axum::body::to_bytes(body, MAX_REWRITTEN_BODY).await {
            Ok(bytes) => {
                let pinned = pin_selection(&bytes, &selected);
                let rewritten = pinned.is_some();
                (reqwest::Body::from(pinned.unwrap_or(bytes)), rewritten)
            }
            Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "The request is too large"),
        }
    } else {
        (
            reqwest::Body::wrap_stream(body.into_data_stream().map_err(std::io::Error::other)),
            false,
        )
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
    let body = if lists_models && status.is_success() {
        match response.bytes().await {
            Ok(bytes) => {
                Body::from(account_models(&bytes, selected.model.as_deref()).unwrap_or(bytes))
            }
            Err(_) => return error(StatusCode::BAD_GATEWAY, "The model list was cut off"),
        }
    } else {
        Body::from_stream(response.bytes_stream())
    };
    out.body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// The hub's model list cut to the account's models and `selected`.
///
/// `agy` offers at most 40 of the ids it is given, so on the full hub list a
/// published model can be missing and fail as `Unknown model ID`.
fn account_models(body: &[u8], selected: Option<&str>) -> Option<Bytes> {
    let mut json: serde_json::Value = serde_json::from_slice(body).ok()?;
    json.get_mut("data")?.as_array_mut()?.retain(|row| {
        row["owned_by"]
            .as_str()
            // The hub's owner for the Antigravity account's own models.
            .is_some_and(|owner| owner.eq_ignore_ascii_case("antigravity"))
            || row["id"].as_str().is_some_and(|id| Some(id) == selected)
    });
    serde_json::to_vec(&json).ok().map(Bytes::from)
}

/// `body` pinned to `selected`, when it names the selected model with or
/// without its effort suffix; `None` leaves the body as sent.
fn pin_selection(body: &[u8], selected: &Selection) -> Option<Bytes> {
    let model = selected.model.as_deref()?;
    let mut json: serde_json::Value = serde_json::from_slice(body).ok()?;
    let sent = json.get("model")?.as_str()?;
    let names_selected =
        sent == model || moatless_types::model::strip_hub_effort_suffix(model) == sent;
    if !names_selected {
        return None;
    }
    json["model"] = model.into();
    if let Some(effort) = &selected.effort {
        json["reasoning_effort"] = effort.as_str().into();
    }
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
    fn a_request_for_the_selected_model_runs_it_at_the_selected_effort() {
        let pinned = |selected: &Selection, sent: &str| {
            let body = serde_json::json!({"model": sent, "reasoning_effort": "medium"});
            pin_selection(&serde_json::to_vec(&body).unwrap(), selected).map(|b| {
                let json: serde_json::Value = serde_json::from_slice(&b).unwrap();
                (json["model"].clone(), json["reasoning_effort"].clone())
            })
        };
        let argon = |effort: Option<&str>| Selection {
            model: Some("gemini-4-argon-high".into()),
            effort: effort.map(str::to_string),
        };
        assert_eq!(
            pinned(&argon(Some("low")), "gemini-4-argon"),
            Some(("gemini-4-argon-high".into(), "low".into()))
        );
        assert_eq!(
            pinned(&argon(None), "gemini-4-argon-high"),
            Some(("gemini-4-argon-high".into(), "medium".into()))
        );
        // agy's own utility calls name another model.
        assert_eq!(pinned(&argon(Some("low")), "gemini-3.1-flash-lite"), None);
        assert_eq!(pinned(&Selection::default(), "gemini-4-argon"), None);
        assert_eq!(pin_selection(b"not json", &argon(None)), None);
    }

    #[test]
    fn the_agent_is_shown_the_account_models_and_the_selected_one() {
        let body = serde_json::json!({"object": "list", "data": [
            {"id": "gemini-4-argon-high", "owned_by": "antigravity"},
            {"id": "claude-sonnet-5-5", "owned_by": "anthropic"},
            {"id": "gpt-6-sol", "owned_by": "openai"}
        ]});
        let ids = |selected| {
            let listed = account_models(&serde_json::to_vec(&body).unwrap(), selected).unwrap();
            let json: serde_json::Value = serde_json::from_slice(&listed).unwrap();
            json["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(None), ["gemini-4-argon-high"]);
        assert_eq!(
            ids(Some("claude-sonnet-5-5")),
            ["gemini-4-argon-high", "claude-sonnet-5-5"]
        );
    }
}
