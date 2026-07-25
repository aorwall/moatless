//! Shared utilities for integration tests.
//!
//! Client construction is delegated to [`moatless_cli::client::MoatClient`] —
//! the same client the CLI uses — so tests exercise the real client code path.

pub mod callback_server;
pub mod mock_github;
pub mod raw_api;
pub mod sandbox_helpers;

use moatless_api_schemas::adapters::{AdapterConnectionResponse, CreateAdapterConnectionRequest};
use moatless_api_schemas::enums::RepositoryKind;
use moatless_api_schemas::enums::{AdapterKind, RoutingMode};
use moatless_api_schemas::repository::CreateRepositoryRequest;
use moatless_api_schemas::loops::{
    CreateSubscription, CreateLoopRequest, CreateLoopConfig, CreateLoopSource,
    LoopKind, Loop,
};
use moatless_cli::client::MoatClient;

/// A test user plus a client authenticated as them.
pub struct TestUser {
    /// Server-assigned internal user id (a uuid, in text form).
    pub id: String,
    pub login: String,
    pub client: MoatClient,
}

/// Password used for every user the harness creates. Long enough to satisfy
/// the server's minimum.
const TEST_PASSWORD: &str = "integration-test-password";

/// The bootstrap admin's session cookie, resolved once per test process.
///
/// argon2 verification costs ~100ms, and cargo runs one process per test file,
/// so caching this keeps the per-test login cost off the critical path.
static ADMIN_COOKIE: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();

fn admin_username() -> String {
    std::env::var("ADMIN_USERNAME").unwrap_or_else(|_| "admin".into())
}

fn admin_password() -> String {
    std::env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "ci-admin-password-1234".into())
}

/// Log in and return the raw `name=value` session cookie.
pub async fn login_cookie(username: &str, password: &str) -> String {
    let resp = reqwest::Client::new()
        .post(format!("{}/api/v1/auth/password/login", base_url()))
        .json(&serde_json::json!({ "username": username, "password": password }))
        .send()
        .await
        .expect("password login request failed");

    let status = resp.status();
    assert!(
        status.is_success(),
        "password login for '{username}' failed with {status}: {}",
        resp.text().await.unwrap_or_default()
    );

    resp.headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|raw| {
            let pair = raw.split(';').next()?.trim();
            pair.starts_with("moatless_session=").then(|| pair.to_string())
        })
        .expect("login response set no moatless_session cookie")
}

/// A client authenticated as the bootstrap admin.
pub async fn admin_client() -> MoatClient {
    let cookie = ADMIN_COOKIE
        .get_or_init(|| async { login_cookie(&admin_username(), &admin_password()).await })
        .await;
    MoatClient::new_with_session_cookie(&base_url(), cookie)
}

/// The default test client — an authenticated admin.
pub async fn test_client() -> MoatClient {
    admin_client().await
}

/// A client with no credentials, for asserting that endpoints reject anonymous
/// callers.
pub fn unauthenticated_client() -> MoatClient {
    MoatClient::new_unauthenticated().unwrap()
}

/// Create a user through the admin API and return a client logged in as them.
pub async fn client_as_user(login: &str, role: &str) -> TestUser {
    client_as_user_with_teams(login, role, &[]).await
}

/// Create a user, add them to `team_ids`, and return a client logged in as them.
///
/// The login happens **after** the team memberships are written: team ids are
/// baked into the session JWT at login and never re-resolved per request, so a
/// client created before `add_member` would carry an empty `team_ids`.
pub async fn client_as_user_with_teams(login: &str, role: &str, team_ids: &[&str]) -> TestUser {
    let admin = admin_client().await;
    // Idempotent: tests seed the same logins repeatedly within a process, and
    // `reset_database()` may or may not have run in between.
    let created_id = match admin
        .api()
        .create_user(&moatless_api_schemas::auth::CreateUserRequest {
            is_bot: false,
            github_app_key: None,
            login: Some(login.to_string()),
            password: Some(TEST_PASSWORD.to_string()),
            role: Some(parse_role(role)),
            must_change_password: Some(false),
        })
        .await
    {
        Ok(created) => Some(created.id.to_string()),
        Err(e) if is_conflict(&e) => None,
        Err(e) => panic!("failed to create test user '{login}': {e}"),
    };

    // The internal id is a server-minted uuid. On the conflict path resolve it by
    // logging in and reading `me()` rather than reconstructing a provider handle.
    let user_id = match created_id {
        Some(id) => id,
        None => {
            let cookie = login_cookie(login, TEST_PASSWORD).await;
            let probe = MoatClient::new_with_session_cookie(&base_url(), &cookie);
            probe
                .api()
                .me()
                .await
                .ok()
                .and_then(|session| session.user)
                .map(|user| user.id.to_string())
                .unwrap_or_else(|| panic!("failed to resolve id for existing test user '{login}'"))
        }
    };

    for team_id in team_ids {
        admin
            .api()
            .add_member(
                team_id,
                &moatless_api_schemas::team::AddTeamMemberRequest {
                    user_id: user_id.clone(),
                    role: Some("member".into()),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("failed to add '{login}' to team '{team_id}': {e}"));
    }

    let cookie = login_cookie(login, TEST_PASSWORD).await;
    TestUser {
        id: user_id,
        login: login.to_string(),
        client: MoatClient::new_with_session_cookie(&base_url(), &cookie),
    }
}

fn is_conflict(error: &moatless_api_client::ApiError) -> bool {
    matches!(error, moatless_api_client::ApiError::Api { status: 409, .. })
}

/// Log a user back in, picking up team memberships granted since their last
/// login. Required after any membership mutation — see
/// [`client_as_user_with_teams`].
pub async fn relogin(user: &TestUser) -> TestUser {
    let cookie = login_cookie(&user.login, TEST_PASSWORD).await;
    TestUser {
        id: user.id.clone(),
        login: user.login.clone(),
        client: MoatClient::new_with_session_cookie(&base_url(), &cookie),
    }
}

fn parse_role(role: &str) -> moatless_api_schemas::enums::UserRole {
    match role {
        "admin" => moatless_api_schemas::enums::UserRole::Admin,
        _ => moatless_api_schemas::enums::UserRole::User,
    }
}

/// Build a raw reqwest client carrying an existing user's session cookie.
pub async fn raw_client_for(user: &TestUser) -> reqwest::Client {
    raw_client_with_cookie(&login_cookie(&user.login, TEST_PASSWORD).await)
}

/// Build a raw reqwest client carrying the bootstrap admin's session cookie.
pub async fn raw_admin_client() -> reqwest::Client {
    raw_client_with_cookie(&login_cookie(&admin_username(), &admin_password()).await)
}

/// Build a raw reqwest client that sends `cookie` on every request.
pub fn raw_client_with_cookie(cookie: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::COOKIE, cookie.parse().unwrap());
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

/// Create a user and return them alongside a raw reqwest client bearing their
/// session cookie.
pub async fn raw_client_as_user(login: &str, role: &str) -> (TestUser, reqwest::Client) {
    let user = client_as_user(login, role).await;
    let http = raw_client_for(&user).await;
    (user, http)
}

/// Return the base URL for raw API requests.
pub fn base_url() -> String {
    std::env::var("MOATLESS_API_URL").unwrap_or_else(|_| "http://localhost:8080".into())
}

/// Agent type override for agent-tier tests (env: `E2E_AGENT_TYPE`).
///
/// Lets the whole agent tier run against an alternate agent transport, e.g.
/// `E2E_AGENT_TYPE=claude-code-cli` to exercise the sandbox's subprocess/NDJSON
/// transport (`SANDBOX_USE_TMUX` is left unset by the backend's agent catalog
/// for that agent type). Unset → `None` → backend default (`claude-code`, which
/// runs over the subprocess/NDJSON transport; `claude-code-tui` opts into the
/// persistent tmux session).
pub fn e2e_agent_type() -> Option<moatless_api_schemas::shared::AgentType> {
    std::env::var("E2E_AGENT_TYPE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(moatless_api_schemas::shared::AgentType)
}

/// Whether the agent transport under test reports real cost data.
///
/// The tmux transport has no machine-readable cost stream, so `total_cost_usd`
/// stays `0.0` — cost assertions must be skipped there. Only the subprocess/NDJSON
/// transports report cost: the default (`claude-code`, i.e. `None`),
/// `claude-code-cli`, and `codex`. The tmux-backed `claude-code-tui` and
/// `claude-code-fable` do not.
pub fn e2e_agent_reports_cost() -> bool {
    matches!(
        e2e_agent_type().as_ref().map(|t| t.as_str()),
        None | Some("claude-code") | Some("claude-code-cli") | Some("codex")
    )
}

/// Run an arbitrary SQL statement via `docker exec moatless-postgres psql`.
///
/// Useful for tests that need to set up rows the public API does not expose
/// (e.g. stamping `tasks.trigger_id` directly to simulate a scheduler tick).
pub fn exec_psql(sql: &str) {
    let output = std::process::Command::new("docker")
        .args([
            "exec",
            "moatless-postgres",
            "psql",
            "-U",
            "moatless",
            "-d",
            "moatless",
            "-c",
            sql,
        ])
        .output()
        .expect("failed to run docker exec for psql");

    assert!(
        output.status.success(),
        "psql exec failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run a read-only query and return psql's stdout (tuples-only, unaligned).
/// Used to assert on how a value is *stored*, not just what the API echoes back.
pub fn query_psql(sql: &str) -> String {
    let output = std::process::Command::new("docker")
        .args([
            "exec",
            "moatless-postgres",
            "psql",
            "-U",
            "moatless",
            "-d",
            "moatless",
            "-tAc",
            sql,
        ])
        .output()
        .expect("failed to run docker exec for psql");

    assert!(
        output.status.success(),
        "psql query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The standard test users, each with a client logged in as them.
pub struct TestUsers {
    pub alice: TestUser,
    pub bob: TestUser,
    pub carol: TestUser,
}

/// Create the standard test users (alice, bob, carol) via the admin API.
pub async fn seed_test_users() -> TestUsers {
    TestUsers {
        alice: client_as_user("alice", "user").await,
        bob: client_as_user("bob", "user").await,
        carol: client_as_user("carol", "admin").await,
    }
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
        scope: None,
        team_id: None,
    }
}

/// Ensure a template repository exists for tests that need one. Returns the repository ID.
pub async fn ensure_template_repo(client: &MoatClient) -> String {
    const TEMPLATE_URL: &str = "https://github.com/aorwall/hello-world-ui.git";

    let repos = client.api().list_repositories().await.unwrap();
    if let Some(repo) = repos.iter().find(|r| {
        r.remote_url
            .as_deref()
            .is_some_and(|url| canonical_repo_url(url) == canonical_repo_url(TEMPLATE_URL))
    }) {
        return repo.id.clone();
    }
    let mut req = create_repo_request("E2E Template Repository", RepositoryKind::Template);
    req.remote_url = Some(TEMPLATE_URL.into());
    match client.api().create_repository(&req).await {
        Ok(repo) => repo.id,
        Err(error) if is_conflict(&error) => {
            client
                .api()
                .list_repositories()
                .await
                .unwrap()
                .into_iter()
                .find(|repo| {
                    repo.remote_url.as_deref().is_some_and(|url| {
                        canonical_repo_url(url) == canonical_repo_url(TEMPLATE_URL)
                    })
                })
                .unwrap_or_else(|| panic!("template URL conflicted but no repository is visible"))
                .id
        }
        Err(error) => panic!("failed to create template repository: {error}"),
    }
}

fn canonical_repo_url(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// Canonical webhook integration fixture: one integration connection and one
/// unified adapter-event trigger with an embedded rule.
pub struct WebhookIntegrationFixture {
    pub connection: AdapterConnectionResponse,
    pub trigger: Loop,
}

pub async fn create_webhook_integration(
    client: &MoatClient,
    repository_id: &str,
    source_matcher: &str,
    routing_mode: RoutingMode,
    trigger_config: Option<serde_json::Value>,
) -> WebhookIntegrationFixture {
    let connection = client
        .api()
        .create_adapter_connection(&CreateAdapterConnectionRequest {
            adapter_kind: AdapterKind::Webhook,
            connection_kind: Some("incoming_webhook".into()),
            external_account_id: source_matcher.into(),
            webhook_secret: None,
            source_name: None,
            transport_config: None,
            team_id: None,
            scope: None,
        })
        .await
        .unwrap();

    let trigger = client
        .api()
        .create_trigger(&CreateLoopRequest {
            name: Some(source_matcher.into()),
            kind: LoopKind::AdapterEvent,
            active: Some(true),
            scope: None,
            team_id: None,
            config: CreateLoopConfig {
                repository_id: repository_id.into(),
                routing_mode,
                ongoing_task_id: None,
                task_name: None,
                prompt: None,
                tag_ids: None,
                run_as_user_id: None,
                agent_type: None,
                inactive_ttl_days: None,
            },
            source: CreateLoopSource::AdapterEvent(CreateSubscription {
                adapter_kind: AdapterKind::Webhook,
                connection_id: connection.id.clone(),
                source_matcher: source_matcher.into(),
                source_name: Some(source_matcher.into()),
                event_filter: None,
                adapter_config: trigger_config,
                allow_direct_send: Some(false),
                close_task_on_close: Some(true),
                notify_on_success: Some(false),
            }),
        })
        .await
        .unwrap();

    WebhookIntegrationFixture {
        connection,
        trigger,
    }
}

/// Truncate all application tables. Requires docker CLI and running `moatless-postgres` container.
///
/// Mirrors the TypeScript `tests/api/helpers/db-reset.ts` behavior.
pub fn reset_database() {
    let tables = [
        "task_message_content_blocks",
        "task_message_contexts",
        "task_messages",
        "task_events",
        "task_tags",
        "tasks",
        "repository_plugin_repos",
        "plugin_repos",
        "repositories",
        "trigger_receipts",
        "task_bindings",
        "adapter_event_trigger_sources",
        "schedule_trigger_sources",
        "trigger_rules",
        "triggers",
        "integration_connections",
        "integration_pairing_requests",
        "team_members",
        "teams",
        "sessions",
        "api_keys",
        "provider_tokens",
        "secret_audit_events",
        "secrets",
        "file_metadata",
        "tags",
    ];
    // `users` is deliberately not truncated: the bootstrap admin is seeded
    // only at startup, so wiping it would 401 every subsequent login with no
    // recovery short of restarting the backend. Delete everyone else instead
    // — the FK cascade takes their `user_credentials` with them.
    let admin = admin_username().replace('\'', "''");
    let sql = format!(
        "TRUNCATE {} CASCADE; DELETE FROM users WHERE login <> '{admin}';",
        tables.join(", ")
    );
    let output = std::process::Command::new("docker")
        .args([
            "exec",
            "moatless-postgres",
            "psql",
            "-U",
            "moatless",
            "-d",
            "moatless",
            "-c",
            &sql,
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            eprintln!("Database reset: truncated {} tables", tables.len());
        }
        Ok(out) => {
            eprintln!(
                "Database reset warning: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Err(e) => {
            eprintln!("Database reset skipped (docker not available): {e}");
        }
    }
}
