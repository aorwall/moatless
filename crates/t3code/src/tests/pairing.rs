//! The pairing exchange and the ticket route, over HTTP, as a client on another
//! device drives them.

use super::fixtures::*;
use super::*;
use crate::auth::verify_ws_ticket_jwt;
use axum::body::Body;
use axum::http::Request;
use chrono::Utc;
use tower::ServiceExt;

const TEST_SECRET: &str = "test-secret";

fn state_pairing(pairing: DevicePairing) -> UiRpcState {
    UiRpcState {
        pairing: Arc::new(pairing),
        ..state()
    }
}

fn exchange(body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// What the T3 mobile client sends, byte for byte — captured off
/// `bootstrapRemoteBearerSession`, since a hand-written fixture gets the
/// encoding wrong (`+` for space, `%3A` for colons).
fn mobile_exchange(code: &str) -> Request<Body> {
    exchange(&format!(
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange\
         &subject_token={code}\
         &subject_token_type=urn%3At3%3Aparams%3Aoauth%3Atoken-type%3Aenvironment-bootstrap\
         &requested_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token\
         &scope=orchestration%3Aread+orchestration%3Aoperate+terminal%3Aoperate+review%3Awrite+relay%3Aread\
         &client_label=T3+Code+Mobile&client_device_type=mobile&client_os=iOS"
    ))
}

async fn json(response: Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), MAX_FRAME_BYTES)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn a_typed_code_becomes_the_token_the_device_keeps() {
    let pairing = test_pairing(Some(pairing_code("ABCDEFGHJKLM")));

    let response = router(state_pairing(pairing))
        .oneshot(mobile_exchange("ABCDEFGHJKLM"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = json(response).await;
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(
        body["issued_token_type"],
        "urn:ietf:params:oauth:token-type:access_token"
    );
    assert!(body["access_token"].as_str().unwrap().starts_with("mvk_"));
    // The overlap between what it asked for and what we have.
    assert_eq!(
        body["scope"],
        "orchestration:read orchestration:operate terminal:operate review:write relay:read"
    );
    assert!(body["expires_in"].as_i64().unwrap() > 0);
}

#[tokio::test]
async fn a_code_that_cannot_be_redeemed_is_refused_the_way_t3_reads() {
    // Never minted, already spent, expired — one answer for all three.
    let pairing = test_pairing(None);

    let response = router(state_pairing(pairing))
        .oneshot(mobile_exchange("ABCDEFGHJKLM"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = json(response).await;
    assert_eq!(body["_tag"], "EnvironmentAuthInvalidError");
    assert_eq!(body["code"], "auth_invalid");
    assert_eq!(body["reason"], "invalid_credential");
    assert!(!body["traceId"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn a_grant_this_environment_does_not_offer_never_reaches_the_code() {
    let response = router(state_pairing(test_pairing(Some(pairing_code(
        "ABCDEFGHJKLM",
    )))))
    .oneshot(exchange(
        "grant_type=authorization_code&subject_token=ABCDEFGHJKLM\
             &subject_token_type=urn%3At3%3Aparams%3Aoauth%3Atoken-type%3Aenvironment-bootstrap\
             &requested_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token",
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = json(response).await;
    assert_eq!(body["_tag"], "EnvironmentRequestInvalidError");
    assert_eq!(body["reason"], "invalid_command");
}

#[tokio::test]
async fn a_paired_client_trades_its_token_for_a_ticket() {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/auth/websocket-ticket")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(viewer());

    let response = router(state()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = json(response).await;
    let claims = verify_ws_ticket_jwt(body["ticket"].as_str().unwrap(), TEST_SECRET).unwrap();
    assert_eq!(claims.sub, viewer().user_id);
    // T3 decodes this as a `DateTimeUtc`, so it has to be an instant.
    assert!(
        body["expiresAt"]
            .as_str()
            .unwrap()
            .parse::<chrono::DateTime<Utc>>()
            .is_ok()
    );
}

#[tokio::test]
async fn a_deployment_with_no_signing_secret_says_so() {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/auth/websocket-ticket")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(viewer());

    let unsigned = UiRpcState {
        session_jwt_secret: None,
        ..state()
    };
    let response = router(unsigned).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        json(response).await["reason"],
        "websocket_ticket_issuance_failed"
    );
}
