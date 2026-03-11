//! Auth verify integration tests.
//!
//! Mirrors: tests/api/chains/auth-verify.spec.ts
//! Spec IDs: AUTH-30 through AUTH-33

use integration_tests::{raw_api, raw_client_as_user};
use pretty_assertions::assert_eq;

// =============================================================================
// Chain 1: ForwardAuth Verification (dev mode)
// =============================================================================

// AUTH-30: In dev mode, even unauthenticated requests pass
#[tokio::test]
async fn auth_verify_no_credentials() {
    let http = reqwest::Client::new();
    let resp = raw_api::verify_auth(&http).await;
    assert_eq!(resp.status().as_u16(), 200);
}

// AUTH-31: Authenticated user headers pass verification
#[tokio::test]
async fn auth_verify_with_user_headers() {
    let http = raw_client_as_user("alice-test", "alice", "user");
    let resp = raw_api::verify_auth(&http).await;
    assert_eq!(resp.status().as_u16(), 200);
}

// AUTH-32: API key header also passes (dev mode allows all)
#[tokio::test]
async fn auth_verify_with_api_key_header() {
    let http = raw_client_as_user("api-key-user", "bot", "user");
    let resp = raw_api::verify_auth(&http).await;
    assert_eq!(resp.status().as_u16(), 200);
}

// =============================================================================
// Chain 2: Preview Access Control (dev mode)
// =============================================================================

// AUTH-33: Verify with X-Forwarded-Host returns 200 (dev mode)
#[tokio::test]
async fn auth_verify_with_forwarded_host() {
    let http = raw_client_as_user("alice-test", "alice", "user");
    let resp = raw_api::verify_auth_with_headers(
        &http,
        &[(
            "X-Forwarded-Host".into(),
            "some-task--3000.preview.example.com".into(),
        )],
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
}
