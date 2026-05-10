//! Shared test helpers for integration tests.

use std::sync::Arc;

use async_trait::async_trait;
use axum::response::sse::Event;
use futures::stream::BoxStream;
use sqlx::PgPool;

use moatless_vibe_backend::command::models::Command;
use moatless_vibe_backend::config::{
    AuthConfig, CodexOAuthConfig, Config, DatabaseConfig, DockerConfig, GithubPrConfig,
    LinearConfig, ProxyConfig, RedisConfig, S3Config, SandboxConfig, SandboxProviderType,
    SecretsConfig, ServerConfig, SkillConfig, SlackConfig, SseConfig, TelegramConfig,
};
use moatless_vibe_backend::message::models::{MessageRole, MessageType, TaskMessage};
use moatless_vibe_backend::repository::models::{Repository, RepositoryKind, RepositoryProvider};
use moatless_vibe_backend::sandbox::client::SandboxClient;
use moatless_vibe_backend::sandbox::error::SandboxError;
use moatless_vibe_backend::sandbox::models::*;
use moatless_vibe_backend::sandbox::provider::SandboxProvider;
use moatless_vibe_backend::sandbox::schemas::*;
use moatless_vibe_backend::shared_models::AgentType;
use moatless_vibe_backend::state::AppState;
use moatless_vibe_backend::task::models::{Task, TaskStatus, Visibility};

// =========================================================================
// Mock sandbox provider
// =========================================================================

/// Minimal mock that returns errors for all sandbox operations.
/// Tests that exercise DB-only handlers don't hit the sandbox provider.
pub struct MockSandboxProvider;

#[async_trait]
impl SandboxProvider for MockSandboxProvider {
    async fn provision(
        &self,
        _task_id: &str,
        _request: &ProvisionRequest,
    ) -> Result<ProvisionResponse, SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn sandbox_status(
        &self,
        _task_id: &str,
        _server_configs: &[moatless_vibe_backend::sandbox::models::ServerConfig],
    ) -> Result<SandboxStatusFullResponse, SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn stop(&self, _task_id: &str) -> Result<(), SandboxError> {
        Ok(()) // best-effort in handlers
    }

    async fn restart(&self, _task_id: &str) -> Result<(), SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn redeploy(
        &self,
        _task_id: &str,
        _request: &ProvisionRequest,
    ) -> Result<ProvisionResponse, SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn cleanup(&self, _task_id: &str) -> Result<(), SandboxError> {
        Ok(()) // best-effort in handlers
    }

    async fn container_statuses(
        &self,
        _task_id: &str,
    ) -> Result<Vec<ContainerObservedStatus>, SandboxError> {
        Ok(vec![])
    }

    async fn server_logs(
        &self,
        _task_id: &str,
        _server_name: &str,
        _previous: bool,
    ) -> Result<BoxStream<'static, Result<Event, SandboxError>>, SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn sandbox_client(&self, _task_id: &str) -> Result<SandboxClient, SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }
}

// =========================================================================
// Config + AppState factories
// =========================================================================

/// Minimal Config for tests — no env vars needed.
#[allow(dead_code)]
pub fn test_config() -> Config {
    Config {
        server: ServerConfig {
            port: 0,
            metrics_port: 0,
            grpc_port: 50051,
        },
        sandbox_provider: SandboxProviderType::Kubernetes,
        sandbox: SandboxConfig {
            namespace: "test".into(),
            image: "test-sandbox:latest".into(),
            image_pull_policy: "Never".into(),
            grpc_port: 50052,
            resources_cpu_limit: "500m".into(),
            resources_memory_limit: "512Mi".into(),
            resources_cpu_request: "100m".into(),
            resources_memory_request: "128Mi".into(),
            storage_enabled: false,
            storage_size: "1Gi".into(),
            storage_class: None,
            pod_startup_timeout_secs: 10,
            task_token_secret: None,
            file_mount_path: "/tmp/test".into(),
            idle_reaper_enabled: false,
            idle_timeout_minutes: 15,
            idle_check_interval_secs: 300,
            inactive_task_timeout_days: 0,
            max_crash_restart_count: 5,
            node_selector_json: None,
            tolerations_json: None,
            service_account_name: None,
            default_model: None,
        },
        docker: DockerConfig {
            network: "moatless".into(),
            image: "test-sandbox:latest".into(),
            data_dir: "/tmp/moatless-data".into(),
            container_data_dir: "/tmp/moatless-data".into(),
            backend_hostname: "backend".into(),
            enable_socket_mount: false,
            socket_path: "/var/run/docker.sock".into(),
            extra_networks: vec![],
        },
        proxy: ProxyConfig {
            domain: None,
            protocol: "http".into(),
            traefik_auth_middleware_name: "test-auth".into(),
            traefik_auth_middleware_namespace: "test".into(),
            traefik_entry_point: "web".into(),
        },
        database_url: None,
        database: DatabaseConfig {
            url: None,
            max_connections: 5,
        },
        redis: RedisConfig {
            url: None,
            max_stream_length: 100,
        },
        sse: SseConfig {
            broadcast_capacity: 16,
            consumer_id: "test".into(),
        },
        auth: AuthConfig {
            provider: None,
            github_client_id: None,
            github_client_secret: None,
            github_callback_url: None,
            github_oauth_scope: String::new(),
            github_required_org: None,
            auth0_domain: None,
            auth0_client_id: None,
            auth0_client_secret: None,
            auth0_callback_url: None,
            auth0_audience: None,
            auth0_groups_claim: String::new(),
            session_max_age_secs: 3600,
            frontend_url: "http://localhost:3000".into(),
            cookie_domain: None,
            session_cookie_name: "test_session".into(),
            is_production: false,
            cache_ttl_secs: 0,
            session_jwt_secret: None,
            test_api_key: None,
            default_team_slug: None,
            github_app_client_id: None,
            github_app_client_secret: None,
            github_app_callback_url: None,
            github_app_id: None,
            github_app_private_key: None,
            github_app_default_installation_id: None,
        },
        s3: S3Config {
            bucket_name: None,
            key_prefix: "test".into(),
            region: "us-east-1".into(),
        },
        slack: SlackConfig {
            bot_token: None,
            signing_secret: None,
            bot_user_id: None,
        },
        telegram: TelegramConfig {
            bot_token: None,
            mode: "webhook".into(),
        },
        github_pr: GithubPrConfig {
            webhook_secret: None,
            bot_token: None,
        },
        linear: LinearConfig {
            webhook_secret: None,
            api_token: None,
        },
        skill: SkillConfig { cache_ttl_secs: 60 },
        secrets: SecretsConfig {
            encryption_key: Some("0123456789abcdef0123456789abcdef".to_string()),
            kms_key_id: None,
        },
        codex_oauth: CodexOAuthConfig {
            issuer: "https://auth.openai.com".into(),
            client_id: "test-client".into(),
        },
    }
}

/// Build an AppState with a real PgPool and mock sandbox provider.
#[allow(dead_code)]
pub fn test_app_state(pool: PgPool) -> AppState {
    AppState::new(
        pool,
        Arc::new(test_config()),
        Arc::new(MockSandboxProvider),
        None, // events
        None, // s3
        None, // channel_providers
    )
}

// =========================================================================
// Factory functions
// =========================================================================

/// Create a minimal Task for tests.
#[allow(dead_code)]
pub fn make_task_row(task_id: &str, user_id: &str) -> Task {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Task {
        task_id: task_id.to_string(),
        short_id: Some(format!("s-{task_id}")),
        created_at: now.clone(),
        last_message_at: now,
        agent_type: AgentType(AgentType::CLAUDE_CODE.into()),
        repository_id: "repo-1".to_string(),
        status: TaskStatus::Open,
        total_cost_usd: 0.0,
        site_url: None,
        template_id: None,
        vibe: None,
        name: Some(format!("Test task {task_id}")),
        description: None,
        site_id: None,
        site_provider: None,
        branch: Some("main".to_string()),
        created_by: Some(user_id.to_string()),
        visibility: Visibility::Private,
        scope: moatless_vibe_backend::scope::Scope::User,
        team_id: Some(format!("personal-{user_id}")),
        owner_user_id: Some(user_id.to_string()),
        created_at_db: None,
        updated_at: None,
        parent_task_id: None,
        append_system_prompt: None,
        sandbox_started_at: None,
        headless: false,
        ttl_minutes: 15,
        inactive_ttl_days: 7,
        task_template_id: None,
    }
}

/// Create a minimal Repository for tests.
#[allow(dead_code)]
pub fn make_repository_row(id: &str, user_id: &str) -> Repository {
    let now = chrono::Utc::now();
    Repository {
        id: id.to_string(),
        name: format!("repo-{id}"),
        kind: RepositoryKind::Git,
        remote_url: Some("https://github.com/test/repo.git".to_string()),
        default_branch: Some("main".to_string()),
        docker_image: None,
        system_prompt: None,
        setup_commands: None,
        env_vars: None,
        servers: None,
        plugins: None,
        server_start_commands: None,
        startup_timeout_seconds: None,
        build_command: None,
        build_output_directory: None,
        enable_inspector: None,
        auto_reload: None,
        description: None,
        provider: Some(RepositoryProvider::Github),
        scope: moatless_vibe_backend::scope::Scope::User,
        team_id: Some(format!("personal-{user_id}")),
        owner_user_id: Some(user_id.to_string()),
        created_by: Some(user_id.to_string()),
        resources: None,
        deleted: Some(false),
        created_at: Some(now),
        updated_at: Some(now),
    }
}

/// Create a minimal TaskMessage for tests.
#[allow(dead_code)]
pub fn make_message_row(task_id: &str, uuid: &str) -> TaskMessage {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    TaskMessage {
        id: 0, // ignored on insert (serial)
        task_id: task_id.to_string(),
        uuid: uuid.to_string(),
        message_type: MessageType::User,
        user_type: "human".to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id: None,
        turn_number: None,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: now,
        message_role: MessageRole::User,
        message_content: serde_json::json!([{"type": "text", "text": "hello"}]),
        assistant_message_id: None,
        assistant_message_type: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        tool_use_result: None,
        sdk_metadata: None,
        attachments: None,
        agent_session_id: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode: None,
        model: None,
        resume: true,
        sent_at: None,
        created_at: None,
        channel_external_id: None,
        channel_provider: None,
        channel_source_name: None,
        command_name: None,
        source_task_id: None,
        start_fresh: false,
    }
}

/// Create a minimal Command for tests.
#[allow(dead_code)]
pub fn make_command_row(name: &str) -> Command {
    Command {
        name: name.to_string(),
        description: format!("Test command {name}"),
        prompt: format!("Do {name}"),
        conditions: None,
        icon: None,
        variant: None,
        active: Some(true),
        will_not_resume: None,
        agent_mode: None,
    }
}

// =========================================================================
// HTTP test helpers
// =========================================================================

/// Build an HTTP request with auth identity extension.
#[allow(dead_code)]
pub fn authed_request(
    method: &str,
    uri: &str,
    user_id: &str,
) -> axum::http::Request<axum::body::Body> {
    let mut req = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(axum::body::Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(moatless_vibe_backend::auth::identity::AuthIdentity {
            user_id: user_id.to_string(),
            login: user_id.to_string(),
            role: moatless_vibe_backend::auth::models::UserRole::User,
            provider: "test".to_string(),
            group_id: None,
            team_ids: vec![],
        });
    req
}

/// Build an HTTP request with auth identity extension and a JSON body.
#[allow(dead_code)]
pub fn authed_json_request(
    method: &str,
    uri: &str,
    user_id: &str,
    body: serde_json::Value,
) -> axum::http::Request<axum::body::Body> {
    let mut req = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    req.extensions_mut()
        .insert(moatless_vibe_backend::auth::identity::AuthIdentity {
            user_id: user_id.to_string(),
            login: user_id.to_string(),
            role: moatless_vibe_backend::auth::models::UserRole::User,
            provider: "test".to_string(),
            group_id: None,
            team_ids: vec![],
        });
    req
}

/// Extract response body as JSON value.
#[allow(dead_code)]
pub async fn body_json(response: axum::http::Response<axum::body::Body>) -> serde_json::Value {
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}
