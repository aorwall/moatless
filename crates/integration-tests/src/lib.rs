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
use moatless_api_schemas::triggers::{
    CreateAdapterEventTriggerSource, CreateTriggerRequest, CreateTriggerRule, CreateTriggerSource,
    TriggerKind, TriggerResponse,
};
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

/// Agent type override for agent-tier tests (env: `E2E_AGENT_TYPE`).
///
/// Lets the whole agent tier run against an alternate agent transport, e.g.
/// `E2E_AGENT_TYPE=claude-code-cli` to exercise the sandbox's subprocess/NDJSON
/// transport (`SANDBOX_USE_TMUX` is left unset by the backend's agent catalog
/// for that agent type). Unset → `None` → backend default (`claude-code`, which
/// now runs over the persistent tmux session).
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
/// transports report cost: `claude-code-cli` and `codex`. The default
/// (`claude-code`, i.e. `None`) and `claude-code-fable` now run over tmux.
pub fn e2e_agent_reports_cost() -> bool {
    matches!(
        e2e_agent_type().as_ref().map(|t| t.as_str()),
        Some("claude-code-cli") | Some("codex")
    )
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
    fn sql_literal(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    let sql = format!(
        r#"
        INSERT INTO users (id, login, provider, role, created_at, last_login_at)
        VALUES ({}, {}, 'integration-test', {}, now(), now())
        ON CONFLICT (id) DO UPDATE SET
            login = EXCLUDED.login,
            role = EXCLUDED.role,
            last_login_at = EXCLUDED.last_login_at
    "#,
        sql_literal(user_id),
        sql_literal(login),
        sql_literal(role),
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
        .output()
        .expect("failed to run docker exec for user seed");

    assert!(
        output.status.success(),
        "failed to seed user {user_id}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
    }
}

/// Ensure a template repository exists for tests that need one. Returns the repository ID.
pub async fn ensure_template_repo(client: &MoatClient) -> String {
    let repos = client.api().list_repositories().await.unwrap();
    if let Some(repo) = repos
        .iter()
        .find(|r| r.kind == RepositoryKind::Template && r.name == "E2E Template Repository")
    {
        return repo.id.clone();
    }
    let mut req = create_repo_request("E2E Template Repository", RepositoryKind::Template);
    req.remote_url = Some("https://github.com/aorwall/hello-world-ui.git".into());
    client.api().create_repository(&req).await.unwrap().id
}

/// Canonical webhook integration fixture: one integration connection and one
/// unified adapter-event trigger with an embedded rule.
pub struct WebhookIntegrationFixture {
    pub connection: AdapterConnectionResponse,
    pub trigger: TriggerResponse,
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
        .create_trigger(&CreateTriggerRequest {
            name: Some(source_matcher.into()),
            kind: TriggerKind::AdapterEvent,
            active: Some(true),
            scope: None,
            team_id: None,
            rule: CreateTriggerRule {
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
            source: CreateTriggerSource::AdapterEvent(CreateAdapterEventTriggerSource {
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
        "users",
        "commands",
        "figma_imports",
        "file_metadata",
        "tags",
    ];
    let sql = format!("TRUNCATE {} CASCADE;", tables.join(", "));
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
