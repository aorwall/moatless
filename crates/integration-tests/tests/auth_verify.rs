//! Auth verify integration tests.
//!
//! Mirrors: tests/api/chains/auth-verify.spec.ts
//! Spec IDs: AUTH-30 through AUTH-35

use integration_tests::{
    client_as_user, ensure_template_repo, raw_admin_client, raw_api, raw_client_as_user,
    raw_client_for, test_client,
};
use moatless_api_schemas::task::CreateTaskRequest;
use pretty_assertions::assert_eq;
use reqwest::header::COOKIE;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

fn task_request(repository_id: String, message: &str) -> CreateTaskRequest {
    CreateTaskRequest {
        repository_id: Some(repository_id),
        workspace_id: None,
        message: Some(message.into()),
        name: None,
        branch: None,
        description: None,
        agent_mode: None,
        agent_type: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
        command_name: None,
        headless: None,
        task_template_id: None,
        parent_task_id: None,
    }
}

fn preview_host(task_id: &str) -> (String, String) {
    (
        "X-Forwarded-Host".into(),
        format!("{task_id}--3000.preview.example.com"),
    )
}

// =============================================================================
// Chain 1: ForwardAuth Verification
// =============================================================================

// AUTH-30: an unauthenticated request is rejected
#[tokio::test]
async fn auth_verify_no_credentials() {
    let resp = raw_api::verify_auth(&reqwest::Client::new()).await;
    assert_eq!(resp.status().as_u16(), 401);
}

// AUTH-30.1: forged X-User-* headers are not an identity — the auth middleware
// strips them before validating.
#[tokio::test]
async fn auth_verify_rejects_forged_user_headers() {
    let resp = raw_api::verify_auth_with_headers(
        &reqwest::Client::new(),
        &[
            ("X-User-Id".into(), "admin".into()),
            ("X-User-Role".into(), "admin".into()),
        ],
    )
    .await;
    assert_eq!(resp.status().as_u16(), 401);
}

// AUTH-31: a real session cookie passes verification
#[tokio::test]
async fn auth_verify_with_session_cookie() {
    let (_user, http) =
        raw_client_as_user(&format!("verify-user-{}", timestamp_suffix()), "user").await;
    let resp = raw_api::verify_auth(&http).await;
    assert_eq!(resp.status().as_u16(), 200);
}

// AUTH-32: a garbage cookie is rejected
#[tokio::test]
async fn auth_verify_with_invalid_cookie() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(COOKIE, "moatless_session=not-a-valid-jwt".parse().unwrap());
    let http = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap();
    let resp = raw_api::verify_auth(&http).await;
    assert_eq!(resp.status().as_u16(), 401);
}

// =============================================================================
// Chain 2: Preview Access Control
//
// Testable for the first time: `verify_handler` no longer short-circuits, so
// the task lookup behind a preview hostname actually runs.
// =============================================================================

// AUTH-33 / AUTH-34: the task's owner reaches its preview host; an unrelated
// authenticated user does not.
#[tokio::test]
async fn auth_verify_preview_host_is_scoped_to_the_task() {
    let suffix = timestamp_suffix();
    let owner = client_as_user(&format!("verify-owner-{suffix}"), "user").await;
    let (_stranger, stranger_http) =
        raw_client_as_user(&format!("verify-stranger-{suffix}"), "user").await;

    let repo_id = ensure_template_repo(&owner.client).await;
    let task = owner
        .client
        .api()
        .create_task(&task_request(repo_id, &format!("Preview task {suffix}")))
        .await
        .unwrap();

    let owner_http = raw_client_for(&owner).await;
    let resp = raw_api::verify_auth_with_headers(&owner_http, &[preview_host(&task.task_id)]).await;
    assert_eq!(resp.status().as_u16(), 200, "owner must reach its preview");

    let resp = raw_api::verify_auth_with_headers(&stranger_http, &[preview_host(&task.task_id)]).await;
    assert_eq!(
        resp.status().as_u16(),
        403,
        "an unrelated user must not reach another task's preview"
    );
}

// AUTH-35: a preview host for a task that does not exist is denied
#[tokio::test]
async fn auth_verify_preview_host_unknown_task_denied() {
    let (_user, http) =
        raw_client_as_user(&format!("verify-nobody-{}", timestamp_suffix()), "user").await;
    let resp = raw_api::verify_auth_with_headers(&http, &[preview_host("no-such-task")]).await;
    assert_eq!(resp.status().as_u16(), 403);
}

// AUTH-35.1: an admin reaches any preview host
#[tokio::test]
async fn auth_verify_preview_host_allows_admin() {
    let suffix = timestamp_suffix();
    let admin = test_client().await;
    let repo_id = ensure_template_repo(&admin).await;
    let task = admin
        .api()
        .create_task(&task_request(repo_id, &format!("Admin preview task {suffix}")))
        .await
        .unwrap();

    let http = raw_admin_client().await;
    let resp = raw_api::verify_auth_with_headers(&http, &[preview_host(&task.task_id)]).await;
    assert_eq!(resp.status().as_u16(), 200);
}
