//! Sharing & access over HTTP: who may mint and revoke, and what the list says.

use super::fixtures::*;
use super::*;
use axum::body::Body;
use axum::http::Request;
use moatless_types::ui_rpc::UiRpcAuthMethod;
use tower::ServiceExt;

fn post(uri: &str, body: &str, method: UiRpcAuthMethod) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request.extensions_mut().insert(viewer());
    request.extensions_mut().insert(method);
    request
}

async fn json(response: Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), MAX_FRAME_BYTES)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn a_browser_session_mints_a_pairing_link() {
    let response = router(state())
        .oneshot(post(
            "/api/auth/pairing-token",
            r#"{"label":" Phone "}"#,
            UiRpcAuthMethod::SessionCookie,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert!(!body["id"].as_str().unwrap().is_empty());
    assert_eq!(body["credential"].as_str().unwrap().len(), 12);
    assert_eq!(body["label"], "Phone");
    assert!(
        body["expiresAt"]
            .as_str()
            .unwrap()
            .parse::<chrono::DateTime<chrono::Utc>>()
            .is_ok()
    );
}

#[tokio::test]
async fn a_paired_device_can_neither_mint_nor_revoke() {
    for (uri, body) in [
        ("/api/auth/pairing-token", "{}"),
        ("/api/auth/pairing-links/revoke", r#"{"id":"pairing-1"}"#),
        ("/api/auth/clients/revoke", r#"{"sessionId":"key-1"}"#),
        ("/api/auth/clients/revoke-others", ""),
    ] {
        for method in [UiRpcAuthMethod::ApiKey, UiRpcAuthMethod::WebSocketTicket] {
            let response = router(state())
                .oneshot(post(uri, body, method))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
            let body = json(response).await;
            assert_eq!(body["_tag"], "EnvironmentScopeRequiredError");
            assert_eq!(body["code"], "insufficient_scope");
            assert_eq!(body["requiredScope"], "access:write");
            assert!(body.get("reason").is_none());
        }
    }
}

#[tokio::test]
async fn a_link_narrower_than_the_grant_is_refused() {
    let response = router(state())
        .oneshot(post(
            "/api/auth/pairing-token",
            r#"{"scopes":["orchestration:read"]}"#,
            UiRpcAuthMethod::SessionCookie,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json(response).await["reason"], "invalid_scope");
}

#[tokio::test]
async fn a_link_asking_for_exactly_the_grant_is_minted() {
    let scopes: Vec<&str> = crate::oauth::GRANTED_SCOPES
        .iter()
        .rev()
        .map(|scope| scope.as_str())
        .collect();
    let response = router(state())
        .oneshot(post(
            "/api/auth/pairing-token",
            &json!({ "scopes": scopes }).to_string(),
            UiRpcAuthMethod::SessionCookie,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_browser_session_unpairs_its_devices() {
    let revoked = router(state())
        .oneshot(post(
            "/api/auth/clients/revoke",
            r#"{"sessionId":"key-1"}"#,
            UiRpcAuthMethod::SessionCookie,
        ))
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::OK);
    assert_eq!(json(revoked).await["revoked"], true);

    let unknown = router(state())
        .oneshot(post(
            "/api/auth/clients/revoke",
            r#"{"sessionId":"key-2"}"#,
            UiRpcAuthMethod::SessionCookie,
        ))
        .await
        .unwrap();
    assert_eq!(json(unknown).await["revoked"], false);

    let all = router(state())
        .oneshot(post(
            "/api/auth/clients/revoke-others",
            "",
            UiRpcAuthMethod::SessionCookie,
        ))
        .await
        .unwrap();
    assert_eq!(json(all).await["revokedCount"], 1);
}

#[tokio::test]
async fn the_list_names_each_link_and_device_as_t3_decodes_them() {
    let pairing = test_pairing(Some(pairing_code("ABCDEFGHJKLM")));
    let snapshot = crate::access::snapshot(&pairing, &Uuid::nil().to_string())
        .await
        .unwrap();

    let link = &snapshot["pairingLinks"][0];
    assert_eq!(link["id"], "pairing-1");
    assert_eq!(link["subject"], Uuid::nil().to_string());
    assert!(
        link["scopes"]
            .as_array()
            .unwrap()
            .contains(&json!("orchestration:read"))
    );

    let device = &snapshot["clientSessions"][0];
    assert_eq!(device["sessionId"], "key-1");
    assert_eq!(device["method"], "bearer-access-token");
    assert_eq!(device["client"]["label"], "T3 Code Mobile (iOS)");
    assert_eq!(device["client"]["os"], "iOS");
    assert_eq!(device["client"]["deviceType"], "mobile");
    assert_eq!(device["lastConnectedAt"], Value::Null);
    assert_eq!(device["current"], false);
}

#[tokio::test]
async fn only_a_browser_session_is_told_it_may_manage_access() {
    let browser = crate::rpc::config::session_state(&state(), Some(UiRpcAuthMethod::SessionCookie));
    assert!(
        browser["scopes"]
            .as_array()
            .unwrap()
            .contains(&json!("access:write"))
    );
    for method in [
        None,
        Some(UiRpcAuthMethod::ApiKey),
        Some(UiRpcAuthMethod::WebSocketTicket),
    ] {
        let device = crate::rpc::config::session_state(&state(), method);
        assert!(
            !device["scopes"]
                .as_array()
                .unwrap()
                .contains(&json!("access:write"))
        );
    }
}
