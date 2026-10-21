//! What `moatless_adapter_api_total` records for each answer Linear gives: one
//! increment per GraphQL request sent, labelled by who is at fault.

use std::future::Future;
use std::sync::Mutex;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::any;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};

use super::error_source_tests::{ISSUE_NOT_FOUND, LINEAR_INTERNAL, RATE_LIMITED, UNKNOWN_FIELD};
use super::*;
use moatless_adapter_model::test_support::{UNREACHABLE_BASE, stub_provider};

/// The 503 Linear's edge proxy returned on 2026-10-20.
const UPSTREAM_RESET: &str = "upstream connect error or disconnect/reset before headers. \
reset reason: connection termination";

const VIEWER: &str = r#"{"data":{"viewer":{"id":"user-1"}}}"#;

/// Captured from `attachmentLinkURL` for a URL another issue already holds.
const ALREADY_LINKED: &str = r#"{"data":null,"errors":[{"message":"Input error","path":["attachmentLinkURL"],"extensions":{"code":"INPUT_ERROR","userError":true,"userPresentableMessage":"This URL has already been linked with OP-3."}}]}"#;

/// A stored app token: `access_token` as given, with a refresh token and no
/// expiry, so only a 401 triggers a refresh.
struct StoredToken(&'static str);

#[async_trait::async_trait]
impl AppCredentials for StoredToken {
    async fn app_token(
        &self,
        _provider: &str,
    ) -> Result<Option<AppToken>, moatless_types::store::StoreError> {
        Ok(Some(AppToken {
            access_token: self.0.to_string(),
            refresh_token: Some("lin_refresh".to_string()),
            token_type: None,
            expires_at: None,
            metadata: None,
        }))
    }

    async fn store_app_token(
        &self,
        _provider: &str,
        _token: &AppToken,
    ) -> Result<(), moatless_types::store::StoreError> {
        Ok(())
    }
}

fn api_at(base: impl AsRef<str>, access_token: &'static str) -> LinearAgentApi {
    LinearAgentApi::with_base_url(
        crate::agent_api::http_client(moatless_core::config::HttpConfig::default().linear),
        Arc::new(StoredToken(access_token)),
        "test-org".to_string(),
        Some("client-id".to_string()),
        Some("client-secret".to_string()),
        base.as_ref(),
    )
}

/// Run `call` and return what it recorded on `moatless_adapter_api_total`, as
/// sorted `(outcome, count)` pairs.
async fn api_outcomes<T>(call: impl Future<Output = T>) -> (T, Vec<(String, u64)>) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let guard = metrics::set_default_local_recorder(&recorder);
    let value = call.await;
    drop(guard);

    let mut outcomes: Vec<(String, u64)> = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, _, _, _)| key.key().name() == "moatless_adapter_api_total")
        .map(|(key, _, _, value)| {
            let label = |name: &str| {
                key.key()
                    .labels()
                    .find(|l| l.key() == name)
                    .map(|l| l.value().to_string())
            };
            assert_eq!(label("adapter").as_deref(), Some("linear"));
            let DebugValue::Counter(count) = value else {
                panic!("moatless_adapter_api_total is a counter");
            };
            (label("outcome").expect("outcome label"), count)
        })
        .collect();
    outcomes.sort();
    (value, outcomes)
}

/// The outcomes one `graphql` call records when Linear answers every request
/// with `status` and `body`.
async fn outcomes_for(status: StatusCode, body: &'static str) -> Vec<(String, u64)> {
    let api = api_at(stub_provider(status, body).await, "lin_test");
    let (_, outcomes) =
        api_outcomes(api.graphql(serde_json::json!({ "query": "{ viewer { id } }" }))).await;
    outcomes
}

fn once(outcome: &str) -> Vec<(String, u64)> {
    vec![(outcome.to_string(), 1)]
}

#[tokio::test]
async fn a_clean_answer_records_success() {
    assert_eq!(outcomes_for(StatusCode::OK, VIEWER).await, once("success"));
}

#[tokio::test]
async fn a_malformed_query_records_client_error() {
    assert_eq!(
        outcomes_for(StatusCode::BAD_REQUEST, UNKNOWN_FIELD).await,
        once("client_error")
    );
}

#[tokio::test]
async fn an_http_200_rejection_of_the_caller_records_client_error() {
    assert_eq!(
        outcomes_for(StatusCode::OK, ISSUE_NOT_FOUND).await,
        once("client_error")
    );
}

#[tokio::test]
async fn a_linear_outage_records_server_error() {
    assert_eq!(
        outcomes_for(StatusCode::SERVICE_UNAVAILABLE, UPSTREAM_RESET).await,
        once("server_error")
    );
}

#[tokio::test]
async fn an_http_200_linear_side_error_records_server_error() {
    assert_eq!(
        outcomes_for(StatusCode::OK, LINEAR_INTERNAL).await,
        once("server_error")
    );
}

/// A rate limit carries every caller-error marker, yet stays retryable.
#[tokio::test]
async fn a_rate_limit_records_server_error() {
    for status in [StatusCode::OK, StatusCode::BAD_REQUEST] {
        assert_eq!(
            outcomes_for(status, RATE_LIMITED).await,
            once("server_error"),
            "{status}"
        );
    }
}

#[tokio::test]
async fn an_undecodable_body_records_server_error() {
    assert_eq!(
        outcomes_for(StatusCode::OK, "<html>not json</html>").await,
        once("server_error")
    );
}

#[tokio::test]
async fn an_unreachable_linear_records_server_error() {
    let api = api_at(UNREACHABLE_BASE, "lin_test");
    let (result, outcomes) =
        api_outcomes(api.graphql(serde_json::json!({ "query": "{ viewer { id } }" }))).await;

    assert!(result.is_err());
    assert_eq!(outcomes, once("server_error"));
}

#[tokio::test]
async fn the_token_exchange_path_records_once() {
    let api = api_at(
        stub_provider(StatusCode::OK, ISSUE_NOT_FOUND).await,
        "lin_test",
    );
    let (result, outcomes) = api_outcomes(api.graphql_with_token(
        "lin_test",
        serde_json::json!({ "query": "{ viewer { id } }" }),
    ))
    .await;

    assert!(result.is_err());
    assert_eq!(outcomes, once("client_error"));
}

/// The already-linked rejection is the idempotent answer `attachment_link_url`
/// asked for, so it is no error at all.
#[tokio::test]
async fn an_already_linked_attachment_records_success() {
    let api = api_at(
        stub_provider(StatusCode::OK, ALREADY_LINKED).await,
        "lin_test",
    );
    let (result, outcomes) =
        api_outcomes(api.attachment_link_url("issue-1", "https://example.com/task", "Task")).await;

    result.expect("already linked is success");
    assert_eq!(outcomes, once("success"));
}

/// Linear that rejects `lin_stale` and accepts the token a refresh returns.
async fn rotating_token_linear(log: Arc<Mutex<Vec<String>>>) -> String {
    async fn handle(
        State(log): State<Arc<Mutex<Vec<String>>>>,
        uri: Uri,
        headers: HeaderMap,
    ) -> (StatusCode, &'static str) {
        log.lock()
            .expect("log is not poisoned")
            .push(uri.path().to_string());
        if uri.path() == "/oauth/token" {
            return (StatusCode::OK, r#"{"access_token":"lin_fresh"}"#);
        }
        match headers.get("authorization").and_then(|v| v.to_str().ok()) {
            Some("Bearer lin_fresh") => (StatusCode::OK, VIEWER),
            _ => (StatusCode::UNAUTHORIZED, r#"{"error":"invalid token"}"#),
        }
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let app = Router::new().fallback(any(handle)).with_state(log);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn a_401_healed_by_refresh_records_auth_error_then_success() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let api = api_at(rotating_token_linear(log.clone()).await, "lin_stale");

    let (result, outcomes) =
        api_outcomes(api.graphql(serde_json::json!({ "query": "{ viewer { id } }" }))).await;

    result.expect("the retry with the fresh token succeeds");
    assert_eq!(
        outcomes,
        vec![("auth_error".to_string(), 1), ("success".to_string(), 1)]
    );
    assert_eq!(
        *log.lock().expect("log is not poisoned"),
        ["/graphql", "/oauth/token", "/graphql"],
        "the refresh call itself is not an API-call outcome"
    );
}
