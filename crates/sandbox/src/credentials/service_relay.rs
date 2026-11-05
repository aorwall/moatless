//! The loopback listener a pod's tools reach a service proxy through.
//!
//! `{SERVICE_RELAY_ORIGIN}/{service}/…` is forwarded to the backend's
//! `/proxy/{service}/…` with the task token current at that moment, so a tool
//! keeps working across token rotation. The relay knows no service list: the
//! backend decides what each Workspace may reach, and injects the credential
//! the pod never holds.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use moatless_types::credential_key::SERVICE_RELAY_ORIGIN;
use moatless_types::http::HttpStreamTimeouts;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::github_proxy::forwards;
use crate::backend_client::BackendClient;

#[derive(Clone)]
pub struct ServiceRelayState {
    pub backend_client: Arc<dyn BackendClient>,
    /// `BACKEND_API_URL` without a trailing `/`. `None` refuses every request.
    pub backend_url: Option<String>,
    pub http: reqwest::Client,
}

impl ServiceRelayState {
    /// Redirects are returned to the tool rather than followed here.
    #[expect(
        clippy::disallowed_methods,
        clippy::expect_used,
        reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
    )]
    pub fn new(
        backend_client: Arc<dyn BackendClient>,
        backend_url: Option<&str>,
        timeouts: HttpStreamTimeouts,
    ) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .read_timeout(timeouts.idle)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("the TLS backend initialises");
        Self {
            backend_client,
            backend_url: backend_url
                .filter(|url| !url.is_empty())
                .map(|url| url.trim_end_matches('/').to_string()),
            http,
        }
    }
}

/// The backend URL `path_and_query` (`/{service}/…`) is relayed to, or `None`
/// for a path naming no service.
fn target(backend_url: &str, path_and_query: &str) -> Option<String> {
    let rest = path_and_query.strip_prefix('/')?;
    let service = rest.split(['/', '?']).next()?;
    if service.is_empty() {
        return None;
    }
    Some(format!("{backend_url}/proxy/{rest}"))
}

async fn relay(State(state): State<ServiceRelayState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let Some(backend_url) = state.backend_url.as_deref() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "This sandbox has no BACKEND_API_URL, so it reaches no service proxy",
        );
    };
    let path_and_query = parts.uri.path_and_query().map_or("/", |p| p.as_str());
    let Some(url) = target(backend_url, path_and_query) else {
        return error(
            StatusCode::NOT_FOUND,
            &format!("Name a service: {SERVICE_RELAY_ORIGIN}/{{service}}/…"),
        );
    };
    let Some(token) = state.backend_client.auth_token() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "This sandbox holds no task token yet; retry once it has started",
        );
    };
    let mut upstream = state
        .http
        .request(parts.method, &url)
        .body(reqwest::Body::wrap_stream(
            body.into_data_stream().map_err(std::io::Error::other),
        ));
    for (name, value) in &parts.headers {
        // The tool's own `Authorization` carries the placeholder it was given.
        if forwards(name) && name != header::AUTHORIZATION {
            upstream = upstream.header(name, value);
        }
    }
    let response = match upstream.bearer_auth(token).send().await {
        Ok(response) => response,
        Err(e) => {
            warn!(error = %e, "A service proxy request did not reach the backend");
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
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to build a service proxy response");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
}

fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({ "message": message })),
    )
        .into_response()
}

pub fn router(state: ServiceRelayState) -> Router {
    Router::new().fallback(relay).with_state(state)
}

/// Serve [`router`] on `addr` until `cancel`.
pub async fn serve(
    state: ServiceRelayState,
    addr: &str,
    cancel: CancellationToken,
) -> Result<tokio::task::JoinHandle<()>, String> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind({addr}): {e}"))?;
    let app = router(state);
    let addr = addr.to_string();
    Ok(tokio::spawn(async move {
        info!(addr, "Service relay started");
        let serve = axum::serve(listener, app.into_make_service());
        tokio::select! {
            res = serve => {
                if let Err(e) = res {
                    warn!(error = %e, "Service relay exited with error");
                }
            }
            () = cancel.cancelled() => info!("Service relay shutting down"),
        }
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use axum::http::HeaderMap;

    use super::*;

    struct RotatingBackend {
        token: Mutex<String>,
    }

    #[async_trait]
    impl BackendClient for RotatingBackend {
        fn auth_token(&self) -> Option<String> {
            Some(self.token.lock().unwrap().clone())
        }
    }

    type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

    async fn recording_backend() -> (String, Seen) {
        let seen: Seen = Arc::default();
        let recorded = Arc::clone(&seen);
        let app = Router::new().fallback(move |uri: axum::http::Uri, headers: HeaderMap| {
            let recorded = Arc::clone(&recorded);
            async move {
                recorded.lock().unwrap().push((
                    uri.to_string(),
                    headers
                        .get(header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string),
                ));
                "ok"
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen)
    }

    #[test]
    fn a_path_naming_a_service_relays_to_the_backend_proxy() {
        assert_eq!(
            target("http://b", "/grafana/api/ds/query?x=1").as_deref(),
            Some("http://b/proxy/grafana/api/ds/query?x=1")
        );
        assert_eq!(target("http://b", "/").as_deref(), None);
        assert_eq!(target("http://b", "/?x").as_deref(), None);
    }

    #[tokio::test]
    async fn the_live_task_token_replaces_the_placeholder_after_rotation() {
        let (backend_url, seen) = recording_backend().await;
        let backend = Arc::new(RotatingBackend {
            token: Mutex::new("task-1".to_string()),
        });
        let state = ServiceRelayState::new(
            backend.clone(),
            Some(&backend_url),
            crate::http_config::SandboxHttpConfig::default().github,
        );
        let client = state.http.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });

        let send = || {
            client
                .get(format!("http://{addr}/grafana/api/health"))
                .bearer_auth("moatless-brokered")
                .send()
        };
        assert_eq!(send().await.unwrap().status(), 200);
        *backend.token.lock().unwrap() = "task-2".to_string();
        assert_eq!(send().await.unwrap().status(), 200);
        let unnamed = client.get(format!("http://{addr}/")).send().await.unwrap();
        assert_eq!(unnamed.status(), 404);

        let seen = seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![
                (
                    "/proxy/grafana/api/health".to_string(),
                    Some("Bearer task-1".to_string())
                ),
                (
                    "/proxy/grafana/api/health".to_string(),
                    Some("Bearer task-2".to_string())
                ),
            ]
        );
    }
}
