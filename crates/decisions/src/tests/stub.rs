//! A loopback stand-in for the Jev API.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use moatless_core::config::{DEFAULT_JEV_MODEL, JevConfig};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

pub(crate) const API_KEY: &str = "test-typesafe-key";
/// Nothing listens on port 1, so a connection is refused at once.
pub(crate) const UNREACHABLE: &str = "http://127.0.0.1:1";

#[derive(Clone)]
pub(crate) struct Reply {
    status: u16,
    retry_after: Option<u64>,
    body: String,
    delay: Duration,
}

impl Reply {
    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            retry_after: None,
            body: json!({"error": "stub"}).to_string(),
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn answer(choice: &str, confidence: f64, probabilities: &[(&str, f64)]) -> Self {
        let probabilities: serde_json::Map<String, Value> = probabilities
            .iter()
            .map(|(name, p)| ((*name).to_string(), json!(p)))
            .collect();
        Self::raw(
            200,
            &json!({
                "answers": {"q": {
                    "type": "choice",
                    "choice": choice,
                    "confidence": confidence,
                    "probabilities": probabilities,
                }},
                "usage": {"input_tokens": 812, "output_tokens": 40},
            })
            .to_string(),
        )
    }

    pub(crate) fn raw(status: u16, body: &str) -> Self {
        Self {
            body: body.to_string(),
            ..Self::status(status)
        }
    }

    pub(crate) fn retry_after(self, seconds: u64) -> Self {
        Self {
            retry_after: Some(seconds),
            ..self
        }
    }

    pub(crate) fn delayed(self, delay: Duration) -> Self {
        Self { delay, ..self }
    }
}

pub(crate) struct Recorded {
    pub(crate) authorization: Option<String>,
    pub(crate) body: Value,
}

#[derive(Default)]
struct Shared {
    replies: Vec<Reply>,
    requests: Mutex<Vec<Recorded>>,
}

/// Serves `/v1/systemone` from a reply script; aborts its server on drop.
pub(crate) struct JevStub {
    pub(crate) base_url: String,
    shared: Arc<Shared>,
    handle: JoinHandle<()>,
}

impl JevStub {
    /// Answers the nth request with `replies[n]`, repeating the last reply.
    pub(crate) async fn start(replies: Vec<Reply>) -> Self {
        let shared = Arc::new(Shared {
            replies,
            requests: Mutex::default(),
        });
        let router = axum::Router::new()
            .route("/v1/systemone", post(serve))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback stub");
        let base_url = format!("http://{}", listener.local_addr().expect("stub address"));
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });
        Self {
            base_url,
            shared,
            handle,
        }
    }

    pub(crate) fn hits(&self) -> usize {
        self.requests().len()
    }

    pub(crate) fn requests(&self) -> std::sync::MutexGuard<'_, Vec<Recorded>> {
        self.shared
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn config(&self) -> JevConfig {
        jev_config(&self.base_url)
    }
}

impl Drop for JevStub {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub(crate) fn jev_config(base_url: &str) -> JevConfig {
    JevConfig {
        api_key: API_KEY.to_string(),
        base_url: base_url.to_string(),
        model: DEFAULT_JEV_MODEL.to_string(),
    }
}

async fn serve(State(shared): State<Arc<Shared>>, headers: HeaderMap, body: String) -> Response {
    let reply = {
        let mut requests = shared
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        requests.push(Recorded {
            authorization: headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
            body: serde_json::from_str(&body).unwrap_or(Value::Null),
        });
        let index = (requests.len() - 1).min(shared.replies.len() - 1);
        shared.replies[index].clone()
    };
    tokio::time::sleep(reply.delay).await;
    let status = StatusCode::from_u16(reply.status).expect("valid stub status");
    let mut response = (status, reply.body).into_response();
    if let Some(seconds) = reply.retry_after {
        response
            .headers_mut()
            .insert("retry-after", seconds.to_string().parse().expect("header"));
    }
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().expect("header"));
    response
}
