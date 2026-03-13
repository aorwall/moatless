//! Shared utilities for integration tests.
//!
//! Client construction is delegated to [`moatless_cli::client::MoatClient`] —
//! the same client the CLI uses — so tests exercise the real client code path.

pub mod callback_server;
pub mod raw_api;
pub mod sandbox_helpers;

use moatless_api_schemas::enums::RepositoryKind;
use moatless_api_schemas::repository::CreateRepositoryRequest;
use moatless_cli::client::MoatClient;

/// Create a test client with no auth headers.
pub fn test_client() -> MoatClient {
    MoatClient::new_unauthenticated().unwrap()
}

/// Create a test client that simulates a specific user via ForwardAuth headers.
pub fn client_as_user(user_id: &str, login: &str, role: &str) -> MoatClient {
    MoatClient::new_as_user(user_id, login, role).unwrap()
}

/// Create a test client with user identity and team membership.
pub fn client_as_user_with_teams(
    user_id: &str,
    login: &str,
    role: &str,
    team_ids: &[&str],
) -> MoatClient {
    MoatClient::new_as_user_with_teams(user_id, login, role, team_ids).unwrap()
}

/// Return the base URL for raw API requests.
pub fn base_url() -> String {
    std::env::var("MOATLESS_API_URL").unwrap_or_else(|_| "http://localhost:8080".into())
}

/// Build a raw reqwest client with ForwardAuth headers for a specific user.
pub fn raw_client_as_user(user_id: &str, login: &str, role: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("X-User-Id", user_id.parse().unwrap());
    headers.insert("X-User-Login", login.parse().unwrap());
    headers.insert("X-User-Role", role.parse().unwrap());
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

/// Seed a test user by making a request that triggers user creation via ForwardAuth headers.
pub async fn seed_user(user_id: &str, login: &str, role: &str) {
    let client = client_as_user(user_id, login, role);
    let _ = client.api().list_teams().await;
}

/// Seed the standard test users (alice, bob, carol).
pub async fn seed_test_users() {
    seed_user("alice-test", "alice", "user").await;
    seed_user("bob-test", "bob", "user").await;
    seed_user("carol-admin", "carol", "admin").await;
}

/// Create a `CreateRepositoryRequest` with only the commonly-used fields set.
pub fn create_repo_request(name: &str, kind: RepositoryKind) -> CreateRepositoryRequest {
    CreateRepositoryRequest {
        name: name.into(),
        kind,
        remote_url: None,
        description: None,
        default_branch: None,
        provider: None,
        docker_image: None,
        auto_reload: None,
        build_command: None,
        build_output_directory: None,
        enable_inspector: None,
        env_vars: None,
        plugins: None,
        resources: None,
        scope: None,
        server_start_commands: None,
        servers: None,
        setup_commands: None,
        startup_timeout_seconds: None,
        system_prompt: None,
        team_id: None,
        visibility: None,
    }
}

/// Ensure a template repository exists for tests that need one. Returns the repository ID.
pub async fn ensure_template_repo(client: &MoatClient) -> String {
    let repos = client.api().list_repositories().await.unwrap();
    if let Some(repo) = repos.iter().find(|r| r.kind == RepositoryKind::Template) {
        return repo.id.clone();
    }
    let mut req = create_repo_request("E2E Template Repository", RepositoryKind::Template);
    req.remote_url = Some("https://github.com/aorwall/hello-world-ui.git".into());
    client.api().create_repository(&req).await.unwrap().id
}
