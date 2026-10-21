//! What a failed Linear call becomes: the variant, the cause it keeps, and the
//! status a route renders for it.

use std::error::Error as _;

use axum::http::StatusCode;
use axum::response::IntoResponse;

use super::*;
use moatless_adapter_model::error::ProviderDecodeError;
use moatless_adapter_model::test_support::{UNREACHABLE_BASE, response_snapshot, stub_provider};

/// An app-credential port nothing stands behind.
///
/// These tests assert the request path, not the credential read;
/// `graphql_with_token` takes the token as an argument, so it never touches
/// the database.
struct NoAppToken;

#[async_trait::async_trait]
impl AppCredentials for NoAppToken {
    async fn app_token(
        &self,
        _provider: &str,
    ) -> Result<Option<AppToken>, moatless_types::store::StoreError> {
        Ok(None)
    }

    async fn store_app_token(
        &self,
        _provider: &str,
        _token: &AppToken,
    ) -> Result<(), moatless_types::store::StoreError> {
        Ok(())
    }
}

fn offline_pool() -> Arc<dyn AppCredentials> {
    Arc::new(NoAppToken)
}

/// A client whose whole API host is `base`, built the way production builds
/// one — same constructor, same fields, only the host replaced.
fn api_at(base: impl AsRef<str>) -> LinearAgentApi {
    LinearAgentApi::with_base_url(
        crate::agent_api::http_client(moatless_core::config::HttpConfig::default().linear),
        offline_pool(),
        "test-org".to_string(),
        None,
        None,
        base.as_ref(),
    )
}

#[tokio::test]
async fn unreachable_linear_keeps_transport_source() {
    let api = api_at(UNREACHABLE_BASE);

    let err = api
        .graphql_with_token(
            "lin_test",
            serde_json::json!({ "query": "{ viewer { id } }" }),
        )
        .await
        .expect_err("dead loopback port must fail");

    let AdapterError::Shared(SharedAdapterError::ProviderUnreachable {
        ref operation,
        ref source,
    }) = err
    else {
        panic!("expected ProviderUnreachable, got {err:?}");
    };
    assert_eq!(operation, "Linear request");
    assert!(source.is_connect() || source.is_request(), "{source}");
    assert!(err.source().is_some(), "cause must stay reachable");
}

#[tokio::test]
async fn undecodable_linear_body_keeps_serde_source() {
    let base = stub_provider(StatusCode::OK, "<html>not json</html>").await;
    let api = api_at(base);

    let err = api
        .graphql_with_token(
            "lin_test",
            serde_json::json!({ "query": "{ viewer { id } }" }),
        )
        .await
        .expect_err("html body must not decode");

    let AdapterError::Shared(SharedAdapterError::ProviderResponseInvalid {
        ref operation,
        ref source,
    }) = err
    else {
        panic!("expected ProviderResponseInvalid, got {err:?}");
    };
    assert_eq!(operation, "Linear JSON");
    assert!(
        matches!(source, ProviderDecodeError::Json(_)),
        "Linear buffers the body itself, so the cause is a serde error"
    );
    assert!(err.source().is_some(), "cause must stay reachable");
}

/// A Linear 401 is a rejection we classify separately from "unreachable".
#[tokio::test]
async fn linear_401_stays_unauthorized() {
    let base = stub_provider(StatusCode::UNAUTHORIZED, r#"{"error":"invalid token"}"#).await;
    let api = api_at(base);

    let err = api
        .graphql_with_token(
            "lin_test",
            serde_json::json!({ "query": "{ viewer { id } }" }),
        )
        .await
        .expect_err("401 must be an error");

    assert!(
        matches!(
            err,
            AdapterError::Shared(SharedAdapterError::Unauthorized(_))
        ),
        "got {err:?}"
    );
    assert!(err.source().is_none());
}

/// Linear's answer to `issue(id:)` for a key it does not have: HTTP 200, with
/// the rejection in `errors`. Captured from the live API.
pub(super) const ISSUE_NOT_FOUND: &str = r#"{"errors":[{"message":"Entity not found: Issue","locations":[{"line":1,"column":23}],"path":["issue"],"extensions":{"type":"invalid input","code":"INPUT_ERROR","statusCode":400,"userError":true,"userPresentableMessage":"Could not find referenced Issue."}}]}"#;

/// Linear's answer to a query naming a field `Issue` does not have: HTTP 400.
/// Captured from the live API.
pub(super) const UNKNOWN_FIELD: &str = r#"{"errors":[{"message":"Cannot query field \"nonExistentField\" on type \"Issue\".","locations":[{"line":1,"column":32}],"extensions":{"http":{"status":400,"headers":{}},"code":"GRAPHQL_VALIDATION_FAILED","type":"graphql error","userError":true}}]}"#;

/// Constructed, not captured: every caller-error marker set, so only the
/// `RATELIMITED` code keeps it off the 400 path.
pub(super) const RATE_LIMITED: &str = r#"{"errors":[{"message":"Rate limit exceeded","extensions":{"code":"RATELIMITED","type":"ratelimited","statusCode":400,"userError":true,"userPresentableMessage":"Rate limit exceeded."}}]}"#;

/// A failure on Linear's side: no `userError`, a 5xx `statusCode`.
pub(super) const LINEAR_INTERNAL: &str = r#"{"errors":[{"message":"Internal server error","extensions":{"code":"INTERNAL_SERVER_ERROR","type":"internal error","statusCode":500}}]}"#;

/// The status and body a route renders when Linear answers one call with
/// `status` and `body`.
async fn rendered(status: StatusCode, body: &'static str) -> (StatusCode, String) {
    let err = api_at(stub_provider(status, body).await)
        .graphql_with_token(
            "lin_test",
            serde_json::json!({ "query": "{ issue(id: \"SP-0\") { id } }" }),
        )
        .await
        .expect_err("Linear rejected the call");
    response_snapshot(err.into_response()).await
}

#[tokio::test]
async fn a_missing_issue_renders_404_with_linears_message() {
    let (status, body) = rendered(StatusCode::OK, ISSUE_NOT_FOUND).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        r#"{"error":"Not found: Linear: Could not find referenced Issue."}"#
    );
}

#[tokio::test]
async fn an_invalid_query_renders_400_with_linears_message() {
    let (status, body) = rendered(StatusCode::BAD_REQUEST, UNKNOWN_FIELD).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        r#"{"error":"Invalid request: Linear: Cannot query field \"nonExistentField\" on type \"Issue\"."}"#
    );
}

#[tokio::test]
async fn a_bare_400_renders_400_with_its_body() {
    let (status, body) = rendered(StatusCode::BAD_REQUEST, "endpoint deprecated").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("endpoint deprecated"), "{body}");
}

/// A delivery that hits a `BadRequest` or `NotFound` goes terminal, and a rate
/// limit clears on its own.
#[tokio::test]
async fn a_rate_limit_stays_502_whatever_else_it_carries() {
    for status in [StatusCode::OK, StatusCode::BAD_REQUEST] {
        let (rendered_status, body) = rendered(status, RATE_LIMITED).await;
        assert_eq!(rendered_status, StatusCode::BAD_GATEWAY, "{status}: {body}");
    }
}

#[tokio::test]
async fn a_linear_side_error_stays_502() {
    for status in [StatusCode::OK, StatusCode::INTERNAL_SERVER_ERROR] {
        let (rendered_status, body) = rendered(status, LINEAR_INTERNAL).await;
        assert_eq!(rendered_status, StatusCode::BAD_GATEWAY, "{status}: {body}");
    }
}
