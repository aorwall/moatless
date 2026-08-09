//! Shared test helpers for integration tests.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "fixtures assert on invariants a broken fixture must fail loudly on"
)]

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use moatless_backend_core::sandbox::SandboxConnector;
use moatless_backend_core::sandbox::SandboxError;
use moatless_backend::compose::compose;
use moatless_core::config::{
    AuthConfig, AuthProvider, CodexOAuthConfig, Config, DatabaseConfig, DockerConfig,
    GithubPrConfig, LinearConfig, OtelConfig, ProxyConfig, RedisConfig, S3Config, SandboxConfig,
    SandboxProviderType, SecretsConfig, ServerConfig, SkillConfig, SlackConfig, SseConfig,
    TelegramConfig,
};
use moatless_backend::message::models::{MessageRole, MessageType, TaskMessage};
use moatless_backend::repository::models::{Repository, RepositoryKind, RepositoryProvider};
use moatless_backend::sandbox::client::SandboxClient;
use moatless_backend::sandbox::lifecycle::{
    ArchiveRetention, NotifyPort, ObservedStatus, ProvisionDeps, RuntimeOperationContext,
    RuntimeOutcome, SandboxRuntime,
};
use moatless_core::scope::ScopeContext;
use moatless_core::shared_models::AgentType;
use moatless_backend::state::AppState;
use moatless_backend::task::models::{Task, TaskStatus, Visibility};
use moatless_backend::workspace::models::{Workspace, WorkspaceRepo};

// =========================================================================
// Mock sandbox capabilities
// =========================================================================

/// Minimal runtime used by DB-only handler tests.
pub struct MockSandboxRuntime;

#[async_trait]
impl SandboxRuntime for MockSandboxRuntime {
    async fn ensure_running(
        &self,
        _task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_backend::sandbox::lifecycle::RuntimeError> {
        Err(
            moatless_backend::sandbox::lifecycle::RuntimeError::Infrastructure(
                "mock: not implemented".into(),
            ),
        )
    }

    async fn stop(
        &self,
        _task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_backend::sandbox::lifecycle::RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Stopped))
    }

    async fn restart(
        &self,
        _task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_backend::sandbox::lifecycle::RuntimeError> {
        Err(
            moatless_backend::sandbox::lifecycle::RuntimeError::Infrastructure(
                "mock: not implemented".into(),
            ),
        )
    }

    async fn redeploy(
        &self,
        _task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_backend::sandbox::lifecycle::RuntimeError> {
        Err(
            moatless_backend::sandbox::lifecycle::RuntimeError::Infrastructure(
                "mock: not implemented".into(),
            ),
        )
    }

    async fn remove(
        &self,
        _task_id: &str,
        _retention: ArchiveRetention,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_backend::sandbox::lifecycle::RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Removed))
    }
}

pub struct MockSandboxConnector;

#[async_trait]
impl SandboxConnector for MockSandboxConnector {
    async fn sandbox_client(&self, _task_id: &str) -> Result<SandboxClient, SandboxError> {
        Err(SandboxError::Other("mock: not implemented".into()))
    }
}

pub struct MockNotify;

#[async_trait]
impl NotifyPort for MockNotify {
    async fn notify(&self, _task_id: &str) {}
}

// =========================================================================
// Config + AppState factories
// =========================================================================

/// The Linear half of a test config.
///
/// `api_base_url` is the injection seam: `LinearAgentApi` is built deep inside
/// webhook and worker paths that hold only an `AppState`, so the only way to
/// point a workflow's Linear calls at a loopback stub is to hand it in here.
/// Pass a stub's base and no test in this file can reach `api.linear.app`;
/// pass `None` and the client is production's.
#[allow(dead_code)]
pub fn linear_config(webhook_secret: Option<&str>, api_base_url: Option<&str>) -> LinearConfig {
    LinearConfig {
        webhook_secret: webhook_secret.map(str::to_string),
        oauth_client_id: None,
        oauth_client_secret: None,
        oauth_callback_url: None,
        api_base_url: api_base_url.map(str::to_string),
    }
}

/// The `adapter_apps` row every Linear API call resolves its client from.
///
/// Linear app credentials are DB-authoritative: `api_for_connection` builds its
/// client from [`linear_app_config`], which reads `adapter_apps` and returns
/// `None` when no row exists — and every Linear emitter treats that as
/// best-effort and silently skips the call. So a test that seeds only
/// [`linear_config`] makes *no* Linear calls at all, and any API-call budget it
/// asserts is vacuous. Seed this alongside the connection.
///
/// `api_base_url` lives in `config_json` (not the secret rows), which is what
/// points the resolved client at a stub. No `adapter_app_secrets` rows are
/// needed: the OAuth client id/secret are only read to refresh a token, and a
/// seeded token that is not near expiry is never refreshed.
///
/// [`linear_app_config`]: moatless_backend::adapter_apps::linear_app_config
#[allow(dead_code)]
pub async fn seed_linear_app(pool: &sqlx::PgPool, api_base_url: &str) {
    sqlx::query(
        "INSERT INTO adapter_apps \
         (id, adapter_kind, app_key, display_name, enabled, config_json, created_by, updated_by) \
         VALUES ('aa_linear_test', 'linear', 'default', 'Linear (test)', true, $1, $2, $2) \
         ON CONFLICT (adapter_kind, app_key) DO UPDATE SET config_json = $1",
    )
    .bind(serde_json::json!({ "api_base_url": api_base_url }))
    .bind(USER_1)
    .execute(pool)
    .await
    .expect("seed Linear adapter app");
}

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
            run_as_non_root: false,
            shared_data_size: "2Gi".into(),
            build_cache_size: "1536Mi".into(),
            persist_toolchain: false,
            cargo_target_on_build_cache: false,
            storage_enabled: false,
            storage_size: "1Gi".into(),
            storage_class: None,
            default_storage_mode: "ebs".into(),
            node_archive_enabled: false,
            retain_storage_on_close: false,
            pod_startup_timeout_secs: 10,
            task_token_secret: None,
            user_token_ttl_secs: 7 * 24 * 3600,
            bootstrap_token_ttl_secs: 1800,
            emit_legacy_user_token_env: true,
            accept_claimless_tokens: true,
            file_mount_path: "/tmp/test".into(),
            idle_reaper_enabled: false,
            cron_scheduler_enabled: false,
            liveness_enabled: false,
            idle_timeout_minutes: 15,
            busy_extension_max_hours: 4,
            idle_check_interval_secs: 300,
            inactive_task_timeout_days: 0,
            orphan_gc_enabled: false,
            codex_refresher_interval_secs: 0,
            max_crash_restart_count: 5,
            sandbox_resources_cpu_request: "1".into(),
            sandbox_resources_cpu_limit: "4".into(),
            sandbox_resources_memory_request: "2Gi".into(),
            sandbox_resources_memory_limit: "8Gi".into(),
            node_selector_json: None,
            tolerations_json: None,
            service_account_name: None,
            default_model: None,
            default_codex_model: None,
            extra_env: vec![],
            cache_volumes: vec![],
            cache_host_root: "/mnt/cache".into(),
            cache_init_image: "docker:dind".into(),
            docker_dind_image: "docker:dind".into(),
            docker_dind_storage_size: "10Gi".into(),
            docker_dind_cpu_request: "500m".into(),
            docker_dind_cpu_limit: "2".into(),
            docker_dind_memory_request: "1Gi".into(),
            docker_dind_memory_limit: "4Gi".into(),
            egress_proxy_url: None,
            egress_proxy_profiles: vec![],
            egress_proxy_no_proxy: "localhost,.svc.cluster.local".into(),
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
            traefik_ingress_namespaces: vec!["test".into()],
        },
        database_url: None,
        database: DatabaseConfig {
            url: None,
            max_connections: 5,
            run_migrations: true,
        },
        redis: RedisConfig {
            url: None,
            max_stream_length: 100,
            event_handler_enabled: true,
        },
        sse: SseConfig {
            broadcast_capacity: 16,
            consumer_id: "test".into(),
        },
        auth: AuthConfig {
            provider: AuthProvider::Local,
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
            admin_username: "admin".into(),
            admin_password: None,
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
            github_apps: vec![],
        },
        s3: S3Config {
            bucket_name: None,
            key_prefix: "test".into(),
            transcript_prefix: "transcripts".into(),
            workspace_prefix: "workspaces".into(),
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
        linear: linear_config(None, None),
        skill: SkillConfig { cache_ttl_secs: 60 },
        secrets: SecretsConfig {
            encryption_key: Some("0123456789abcdef0123456789abcdef".to_string()),
            kms_key_id: None,
        },
        codex_oauth: CodexOAuthConfig {
            issuer: "https://auth.openai.com".into(),
            client_id: "test-client".into(),
        },
        default_run_as_user_id: None,
        otel: OtelConfig {
            otlp_endpoint: None,
            sampling_ratio: 0.1,
            service_name: "test".into(),
        },
    }
}

/// Build an AppState with a real PgPool and mock sandbox capabilities.
#[allow(dead_code)]
pub fn test_app_state(pool: PgPool) -> AppState {
    test_app_state_with_adapters(
        pool,
        None,
        moatless_backend::state::ProviderHandles::default(),
    )
}

/// Build an AppState carrying an adapter registry, for tests that drive an
/// inbound webhook route — the route resolves its provider through
/// `state.adapters`, so a `None` registry rejects the delivery before any of
/// the behaviour under test runs.
#[allow(
    dead_code,
    reason = "this module is compiled into every test binary; each one uses a subset of it"
)]
pub fn test_app_state_with_adapters(
    pool: PgPool,
    adapters: Option<Arc<moatless_backend::adapters::Adapters>>,
    provider_handles: moatless_backend::state::ProviderHandles,
) -> AppState {
    test_app_state_with_config(pool, adapters, provider_handles, test_config())
}

/// Build an AppState carrying one Linear adapter, for tests that drive an
/// inbound Linear route. The one `Arc` derives both places the adapter has to
/// appear and be the same instance: the migrated webhook handler resolves it
/// through the concrete handle (`state.provider_handles.linear`), and the
/// delivery worker finds it as a trait object in `state.adapters`. Taking the
/// `Arc` rather than a whole `ProviderHandles` is what makes the two agree by
/// construction — there is no way to pass a handle the registry does not get.
/// Linear is the only provider any test builds by hand; the rest reach the
/// registry through `compose::from_config`.
#[allow(
    dead_code,
    reason = "this module is compiled into every test binary; each one uses a subset of it"
)]
pub fn test_app_state_with_linear(
    pool: PgPool,
    linear: Arc<moatless_backend::adapters::providers::linear::adapter::LinearAdapter>,
    config: Config,
) -> AppState {
    let mut registry = moatless_backend::adapters::Adapters::new();
    linear.clone().register_into(&mut registry);
    test_app_state_with_config(
        pool,
        Some(Arc::new(registry)),
        moatless_backend::state::ProviderHandles {
            linear: Some(linear),
            ..Default::default()
        },
        config,
    )
}

/// [`test_app_state_with_adapters`] with the config chosen by the caller.
///
/// The Adapter-side seams that decide which host an outbound call reaches live
/// on `Config` — `linear.api_base_url` is the one this exists for. A workflow
/// that builds its API client from `state.config` cannot be pointed at a stub
/// any other way, and a workflow test that cannot be pointed at a stub reaches
/// the real sender.
#[allow(
    dead_code,
    reason = "this module is compiled into every test binary; each one uses a subset of it"
)]
pub fn test_app_state_with_config(
    pool: PgPool,
    adapters: Option<Arc<moatless_backend::adapters::Adapters>>,
    provider_handles: moatless_backend::state::ProviderHandles,
    config: Config,
) -> AppState {
    let ports = compose(pool.clone(), &config);
    let config = Arc::new(config);
    let connector: Arc<dyn SandboxConnector> = Arc::new(MockSandboxConnector);
    let lifecycle = ports.build_lifecycle(
        pool.clone(),
        ProvisionDeps::new(
            ports.tasks.clone(),
            ports.repositories.clone(),
            ports.provider_tokens.clone(),
            ports.plugins.clone(),
            ports.workspaces.clone(),
            ports.users.clone(),
        ),
        Arc::new(MockSandboxRuntime),
        connector.clone(),
        "mock-sandbox:latest".into(),
        None,
    );
    AppState::new(
        &ports,
        config,
        lifecycle,
        connector,
        Arc::new(MockNotify),
        None, // events
        None, // s3
        adapters,
        provider_handles,
    )
}

// =========================================================================
// Factory functions
// =========================================================================

/// Well-known test user ids. `AuthIdentity::user_id` is now a `Uuid`, so HTTP
/// handler tests must authenticate as a real uuid and seed the matching row's
/// owner with the same value. DB-only DAO tests keep using plain symbolic names
/// (`"user-1"`) since they build their `ScopeContext` directly and never parse a
/// uuid.
#[allow(dead_code)]
pub const USER_1: &str = "11111111-1111-1111-1111-111111111111";
#[allow(dead_code)]
pub const USER_2: &str = "22222222-2222-2222-2222-222222222222";

/// A user-scoped [`ScopeContext`] for `user_id`, matching rows seeded by the
/// factories below (owner = `user_id`, no team memberships). Used to assert DB
/// state directly from HTTP handler tests.
#[allow(dead_code)]
pub fn user_scope(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.to_string(),
        is_admin: false,
    }
}

/// Seed the `teams` row that repository (and other team-scoped) rows FK to.
/// Factories use `personal-{user_id}` as the team id; pass that value.
#[allow(dead_code)]
pub async fn seed_team(pool: &PgPool, team_id: &str) {
    sqlx::query(
        "INSERT INTO teams (id, name, slug) VALUES ($1, $2, $1) ON CONFLICT (id) DO NOTHING",
    )
    .bind(team_id)
    .bind(format!("Team {team_id}"))
    .execute(pool)
    .await
    .expect("seed team row");
}

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
        model: None,
        created_by: Some(user_id.to_string()),
        visibility: Visibility::Private,
        scope: moatless_core::scope::Scope::User,
        owner_user_id: Some(user_id.to_string()),
        created_at_db: None,
        updated_at: None,
        append_system_prompt: None,
        headless: false,
        ttl_minutes: 15,
        inactive_ttl_days: 7,
        loop_id: None,
        workspace_id: None,
        disabled_servers: None,
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
        remote_url: Some(format!("https://github.com/test/{id}.git")),
        default_branch: Some("main".to_string()),
        description: None,
        provider: Some(RepositoryProvider::Github),
        scope: moatless_core::scope::Scope::User,
        owner_user_id: Some(user_id.to_string()),
        created_by: Some(user_id.to_string()),
        access_status: "unchecked".to_string(),
        access_checked_at: None,
        access_error: None,
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
        message_text: Some("hello".to_string()),
        assistant_message_id: None,
        assistant_message_type: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        assistant_usage: None,
        tool_use_result: None,
        tool_response: None,
        sdk_metadata: None,
        attachments: None,
        agent_session_id: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode: None,
        model: None,
        effort: None,
        resume: true,
        sent_at: None,
        created_at: None,
        external_event_id: None,
        external_adapter: None,
        external_subject_key: None,
        external_source_name: None,
        external_author: None,
        sent_by: None,
        source_task_id: None,
        start_fresh: false,
    }
}

// =========================================================================
// Adapter delivery
// =========================================================================

/// Run the real adapter-delivery worker with the Linear handlers registered until
/// nothing is left unfinished, then stop it.
///
/// A Linear delivery is answered once it is durably owned, so nothing has
/// reached the Agent when the route returns; this stands in for the supervised
/// worker a running process has. `worker_state` is deliberately the caller's
/// *second* replica — what executes a record never depends on the process that
/// accepted it.
#[allow(
    dead_code,
    reason = "this module is compiled into every integration test binary and each uses a \
              subset of it, so `expect` would fire in the binaries that do call this"
)]
pub async fn drain_linear_adapter_delivery(pool: &PgPool, worker_state: AppState) {
    let mut handlers = moatless_backend::adapters::delivery::AdapterDeliveryHandlers::new();
    let router = Arc::new(
        moatless_backend::adapters::providers::linear::delivery::LinearInboundRouter::new(
            worker_state.adapter_connections.clone(),
            worker_state.db.clone(),
            worker_state.config.clone(),
            worker_state.loops.clone(),
            worker_state.task_bindings.clone(),
            worker_state.tasks.clone(),
            worker_state.task_creation.clone(),
            worker_state.task_relations.clone(),
            worker_state.message_send.clone(),
            worker_state.sandbox_waker.clone(),
            worker_state.sandbox.clone(),
            worker_state.events.clone(),
            worker_state.linear_active_turns.clone(),
            worker_state.routing.clone(),
        ),
    );
    moatless_backend::adapters::providers::linear::delivery::register(&mut handlers, router);

    let cancel = tokio_util::sync::CancellationToken::new();
    let handle = moatless_backend::adapters::delivery::spawn_adapter_delivery_worker(
        worker_state.adapter_delivery.clone(),
        Arc::new(handlers),
        cancel.clone(),
    );

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut drained = false;
    while tokio::time::Instant::now() < deadline {
        let unfinished: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM adapter_delivery WHERE status IN ('pending', 'claimed')",
        )
        .fetch_one(pool)
        .await
        .expect("read the Adapter delivery backlog");
        if unfinished == 0 {
            drained = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    cancel.cancel();
    handle.await.expect("worker exits cleanly on cancellation");
    assert!(drained, "every admitted record reached a terminal state");
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
        .insert(moatless_backend::auth::identity::AuthIdentity {
            user_id: Uuid::parse_str(user_id)
                .expect("authed_request user_id must be a UUID (use common::USER_1)"),
            login: user_id.to_string(),
            role: moatless_backend::auth::models::UserRole::User,
            provider: "test".to_string(),
            group_id: None,
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
        .insert(moatless_backend::auth::identity::AuthIdentity {
            user_id: Uuid::parse_str(user_id)
                .expect("authed_request user_id must be a UUID (use common::USER_1)"),
            login: user_id.to_string(),
            role: moatless_backend::auth::models::UserRole::User,
            provider: "test".to_string(),
            group_id: None,
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

// =========================================================================
// Workspace factories
// =========================================================================

/// Create a minimal user-scoped Workspace row for tests.
///
/// `kind` starts at `blank`; use `refresh_kind` after adding placements to
/// derive the real label. Run-config columns are left empty — placement and
/// ACL behavior is what the DAO tests exercise.
#[allow(dead_code)]
pub fn make_workspace_row(id: &str, user_id: &str) -> Workspace {
    let now = chrono::Utc::now();
    Workspace {
        id: id.to_string(),
        name: format!("ws-{id}"),
        description: None,
        kind: "blank".to_string(),
        docker_image: None,
        docker_access: None,
        sandbox_service_account: None,
        system_prompt: None,
        setup_commands: None,
        env_vars: None,
        servers: None,
        resources: None,
        network_profile: None,
        source: "manual".to_string(),
        source_repository_id: None,
        source_key: None,
        source_config_path: None,
        scope: moatless_core::scope::Scope::User,
        owner_user_id: Some(user_id.to_string()),
        created_by: Some(user_id.to_string()),
        deleted: Some(false),
        created_at: Some(now),
        updated_at: Some(now),
    }
}

/// Create a `WorkspaceRepo` placement row for tests.
#[allow(dead_code)]
pub fn make_workspace_repo_row(
    id: &str,
    workspace_id: &str,
    repository_id: &str,
    is_primary: bool,
    position: i32,
) -> WorkspaceRepo {
    WorkspaceRepo {
        id: id.to_string(),
        workspace_id: workspace_id.to_string(),
        repository_id: repository_id.to_string(),
        branch: None,
        mount_name: None,
        is_primary,
        position,
        created_at: Some(chrono::Utc::now()),
    }
}
