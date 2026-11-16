//! Retrying a refused connection while the hub restarts.

use std::time::{Duration, Instant};

use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;

use super::*;

const SHORT: ConnectRetry = ConnectRetry {
    first: Duration::from_millis(20),
    budget: Duration::from_millis(400),
};

#[expect(
    clippy::disallowed_methods,
    reason = "test client sets a 1s connect timeout against loopback"
)]
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(1))
        .build()
        .unwrap()
}

/// A loopback port nothing listens on, until the caller binds it again.
async fn closed_port() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap()
}

fn echo() -> Router {
    Router::new().route("/v1/messages", post(|body: String| async move { body }))
}

#[tokio::test]
async fn a_request_waits_out_a_hub_that_is_not_listening_yet() {
    let addr = closed_port().await;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, echo()).await.unwrap();
    });
    let request = upstream_request(
        &client(),
        Method::POST,
        &format!("http://{addr}/v1/messages"),
        &HeaderMap::new(),
        Bytes::from_static(b"{\"model\":\"claude\"}"),
    );

    let response = relay_retrying(request, &SHORT).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"{\"model\":\"claude\"}");
}

#[tokio::test]
async fn a_hub_down_past_the_budget_fails_as_a_connect_error() {
    let addr = closed_port().await;
    let request = upstream_request(
        &client(),
        Method::POST,
        &format!("http://{addr}/v1/messages"),
        &HeaderMap::new(),
        Bytes::new(),
    );
    let started = Instant::now();

    let error = relay_retrying(request, &SHORT).await.unwrap_err();

    assert!(error.is_connect());
    assert!(started.elapsed() >= Duration::from_millis(200));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn an_upstream_answer_is_never_retried_even_a_5xx() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let app = Router::new().route(
        "/v1/messages",
        post(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { StatusCode::BAD_GATEWAY }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let request = upstream_request(
        &client(),
        Method::POST,
        &format!("http://{addr}/v1/messages"),
        &HeaderMap::new(),
        Bytes::new(),
    );

    let response = relay_retrying(request, &SHORT).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn only_an_oversized_body_is_refused_as_too_large() {
    let oversized = Body::from(vec![0u8; MAX_REQUEST_BYTES + 1]);
    let refused = buffered(oversized).await.err().unwrap().into_response();
    assert_eq!(refused.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let broken = Body::from_stream(futures::stream::iter([Err::<Bytes, _>(
        std::io::Error::other("caller went away"),
    )]));
    let refused = buffered(broken).await.err().unwrap().into_response();
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
}
