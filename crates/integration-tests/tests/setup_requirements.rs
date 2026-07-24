//! `GET /api/v1/auth/me` setup-requirement reporting.

use integration_tests::mock_github::{MockGithub, MOCK_LOGIN};
use integration_tests::{base_url, query_psql, raw_client_as_user};
use pretty_assertions::assert_eq;

/// Fixed port the backend's `GITHUB_API_BASE_URL` points at in compose.
const DEFAULT_MOCK_PORT: u16 = 18092;

fn mock_port() -> u16 {
    std::env::var("GITHUB_MOCK_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MOCK_PORT)
}

/// Fetch `/auth/me` and return its `setup` block.
async fn setup_block(http: &reqwest::Client) -> serde_json::Value {
    let response = http
        .get(format!("{}/api/v1/auth/me", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    body["setup"].clone()
}

fn github_requirement(setup: &serde_json::Value) -> &serde_json::Value {
    setup["requirements"]
        .as_array()
        .expect("requirements array")
        .iter()
        .find(|r| r["id"] == "github_token")
        .expect("github_token requirement is always reported")
}

/// Remove every github `provider_tokens` row a user could reach, so the
/// requirement reports `missing` regardless of what other tests left behind.
fn clear_all_github_tokens() {
    query_psql("DELETE FROM provider_tokens WHERE provider = 'github'");
    query_psql("DELETE FROM secrets WHERE key = 'GITHUB_TOKEN'");
}

#[tokio::test]
async fn me_reports_the_github_requirement_as_missing_for_a_fresh_user() {
    clear_all_github_tokens();
    let (_user, http) = raw_client_as_user("setup-fresh", "user").await;

    let setup = setup_block(&http).await;
    assert_eq!(setup["complete"], serde_json::json!(false));

    let requirement = github_requirement(&setup);
    assert_eq!(requirement["status"], serde_json::json!("missing"));
    assert_eq!(requirement["severity"], serde_json::json!("required"));
}

#[tokio::test]
async fn a_non_admin_is_never_offered_the_org_wide_route() {
    clear_all_github_tokens();
    let (_user, http) = raw_client_as_user("setup-nonadmin", "user").await;

    let setup = setup_block(&http).await;
    let actions = github_requirement(&setup)["actions"]
        .as_array()
        .expect("unmet requirements carry actions")
        .clone();

    assert!(
        actions.iter().all(|a| a["kind"] != "admin_route"),
        "non-admin was offered an admin-only action: {actions:?}"
    );
    assert!(actions.iter().any(|a| a["kind"] == "open_settings"));
}

#[tokio::test]
async fn an_admin_is_offered_the_org_wide_route_first() {
    clear_all_github_tokens();
    let (_user, http) = raw_client_as_user("setup-admin", "admin").await;

    let setup = setup_block(&http).await;
    let actions = github_requirement(&setup)["actions"]
        .as_array()
        .expect("unmet requirements carry actions")
        .clone();

    assert_eq!(actions[0]["kind"], serde_json::json!("admin_route"));
    assert_eq!(
        actions[0]["target"],
        serde_json::json!("/admin/integrations/github")
    );
}

#[tokio::test]
#[ignore] // Requires GITHUB_PROXY_E2E=1 + GITHUB_API_BASE_URL on the backend
async fn a_users_own_pat_satisfies_the_requirement() {
    if std::env::var("GITHUB_PROXY_E2E").is_err() {
        eprintln!("Skipping: GITHUB_PROXY_E2E not set");
        return;
    }
    clear_all_github_tokens();
    let _mock = MockGithub::start(mock_port()).await;

    let (_user, http) = raw_client_as_user("setup-own-pat", "user").await;

    let save = http
        .put(format!("{}/api/v1/settings/github/pat", base_url()))
        .json(&serde_json::json!({ "token": "ghp_setup_own" }))
        .send()
        .await
        .unwrap();
    assert_eq!(save.status().as_u16(), 200);

    let setup = setup_block(&http).await;
    assert_eq!(setup["complete"], serde_json::json!(true));
    assert_eq!(
        github_requirement(&setup)["status"],
        serde_json::json!("satisfied")
    );
}

/// A stored PAT lives in the encrypted `secrets` table; its `provider_tokens`
/// row carries only a sentinel. Preflight has to dereference that through the
/// secret service — without one, every PAT-backed user reads as disconnected
/// and task creation is rejected with "GitHub account not connected".
///
/// The repo's remote is deliberately not `github.com`, so preflight resolves
/// and validates the token but skips the repo-access probe (which is pinned to
/// the real `api.github.com` and cannot be mocked).
#[tokio::test]
#[ignore] // Requires GITHUB_PROXY_E2E=1 + GITHUB_API_BASE_URL on the backend
async fn a_pat_backed_user_can_create_a_task_for_a_github_repo() {
    if std::env::var("GITHUB_PROXY_E2E").is_err() {
        eprintln!("Skipping: GITHUB_PROXY_E2E not set");
        return;
    }
    clear_all_github_tokens();
    let _mock = MockGithub::start(mock_port()).await;

    let (_user, http) = raw_client_as_user("preflight-pat", "user").await;
    let base_url = base_url();

    let save = http
        .put(format!("{base_url}/api/v1/settings/github/pat"))
        .json(&serde_json::json!({ "token": "ghp_preflight" }))
        .send()
        .await
        .unwrap();
    assert_eq!(save.status().as_u16(), 200);

    // Confirm the PAT really landed as a reference row — the whole point of
    // the test is that preflight can follow the sentinel.
    let auth_method = query_psql(
        "SELECT auth_method FROM provider_tokens \
         WHERE provider = 'github' AND scope = 'user'",
    );
    assert_eq!(auth_method.trim(), "secret_ref");

    let repo = http
        .post(format!("{base_url}/api/v1/repositories"))
        .json(&serde_json::json!({
            "name": "preflight-pat-repo",
            "kind": "remote",
            "remoteUrl": "https://git.example.com/owner/repo",
            "defaultBranch": "main",
            "provider": "github",
            "servers": [],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(repo.status().as_u16(), 200);
    let repo_id = repo.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .expect("repository id")
        .to_string();

    let task = http
        .post(format!("{base_url}/api/v1/tasks"))
        .json(&serde_json::json!({
            "name": "preflight-pat-task",
            "repositoryId": repo_id,
        }))
        .send()
        .await
        .unwrap();

    assert_ne!(
        task.status().as_u16(),
        403,
        "a connected PAT must not be reported as a missing GitHub account"
    );
}

/// Pins the 403. The codex admin endpoints returned 400 here while their
/// OpenAPI annotations documented 403; this must not be reintroduced.
#[tokio::test]
async fn a_non_admin_cannot_manage_global_github_installations() {
    let (_user, http) = raw_client_as_user("setup-org-forbidden", "user").await;
    let base_url = base_url();

    for (method, path) in [
        ("GET", "/api/v1/admin/settings/github/apps"),
        ("GET", "/api/v1/admin/settings/github/installation"),
        ("PUT", "/api/v1/admin/settings/github/installation"),
        ("DELETE", "/api/v1/admin/settings/github/installation"),
    ] {
        let request = match method {
            "GET" => http.get(format!("{base_url}{path}")),
            "DELETE" => http.delete(format!("{base_url}{path}")),
            _ => http
                .put(format!("{base_url}{path}"))
                .json(&serde_json::json!({
                    "githubAppKey": "default",
                    "installationId": 42
                })),
        };
        let status = request.send().await.unwrap().status().as_u16();
        assert_eq!(status, 403, "{method} {path} should be admin-only");
    }
}
