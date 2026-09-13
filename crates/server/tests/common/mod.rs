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

use moatless_backend::compose::compose;
use moatless_backend::sandbox::client::SandboxClient;
use moatless_backend::sandbox::lifecycle::NotifyPort;
use moatless_backend::state::AppState;
use moatless_core::config::{
    Config, DatabaseConfig, DockerConfig, GithubPrConfig, OtelConfig, ProxyConfig, RedisConfig,
    S3Config, SandboxConfig, ServerConfig, SkillConfig, SlackConfig, SseConfig, TelegramConfig,
};
use moatless_core::events::test_support::RecordingPublisher;
use moatless_ports::sandbox::SandboxError;
use moatless_sandbox_control::compose::LifecycleInputs;
use moatless_sandbox_runtime::operation::ObservedStatus;
use moatless_sandbox_runtime::runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::LinearConfig;
use moatless_types::agent::CodexOAuthConfig;
use moatless_types::auth::{AuthConfig, AuthProvider};
use moatless_types::sandbox::{ArchiveRetention, SandboxProviderType};
use moatless_types::secret::SecretsConfig;
use sandbox_proto::client::SandboxConnector;

// =========================================================================
// Mock sandbox capabilities
// =========================================================================

/// Give a named identity a GitHub credential for calls to a local HTTP fixture.
#[allow(
    dead_code,
    reason = "shared fixtures are compiled into all integration targets"
)]
pub async fn seed_github_token(pool: &PgPool, user_id: &str) {
    sqlx::query(
        "INSERT INTO provider_tokens (user_id, provider, access_token, auth_method, scope, owner_user_id) \
         VALUES ($1, 'github', 'fixture-token', 'manual', 'user', $1)",
    )
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed GitHub credential");
}

/// Seed the workspace required by task and sandbox persistence.
#[allow(
    dead_code,
    reason = "shared fixtures are compiled into all integration targets"
)]
pub async fn seed_workspace(pool: &PgPool, id: &str, user_id: &str) {
    sqlx::query(
        "INSERT INTO workspaces (id, name, scope, owner_user_id) \
         VALUES ($1, $1, 'user', $2) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed workspace");
}

/// Seed a workspace that places `repository_id` as its primary repo, so
/// `WorkspaceResolver::for_repository` resolves it for a loop that names none.
#[allow(
    dead_code,
    reason = "shared fixtures are compiled into all integration targets"
)]
pub async fn seed_workspace_placing(pool: &PgPool, id: &str, repository_id: &str, user_id: &str) {
    seed_workspace(pool, id, user_id).await;
    sqlx::query(
        "INSERT INTO workspace_repos (id, workspace_id, repository_id, is_primary) \
         VALUES ($1 || ':' || $2, $1, $2, true) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("seed workspace placement");
}

/// Minimal runtime used by DB-only handler tests.
pub struct MockSandboxRuntime;

#[async_trait]
impl SandboxRuntime for MockSandboxRuntime {
    async fn ensure_running(
        &self,
        _task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        Err(
            moatless_sandbox_runtime::runtime::RuntimeError::Infrastructure(
                "mock: not implemented".into(),
            ),
        )
    }

    async fn stop(
        &self,
        _task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Stopped))
    }

    async fn restart(
        &self,
        _task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        Err(
            moatless_sandbox_runtime::runtime::RuntimeError::Infrastructure(
                "mock: not implemented".into(),
            ),
        )
    }

    async fn redeploy(
        &self,
        _task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        Err(
            moatless_sandbox_runtime::runtime::RuntimeError::Infrastructure(
                "mock: not implemented".into(),
            ),
        )
    }

    async fn remove(
        &self,
        _task_id: &str,
        _retention: ArchiveRetention,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Removed))
    }

    async fn reclaim_storage(
        &self,
        _task_id: &str,
        _rescue_upload_url: Option<&str>,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Stopped))
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
/// webhook/worker paths holding only an `AppState`, so a stub's base here is
/// the only way to keep a workflow's Linear calls off `api.linear.app`.
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
/// Seed alongside the connection: `api_for_connection` returns `None` when
/// missing and every Linear emitter skips the call best-effort, so a test
/// seeding only [`linear_config`] makes no calls and any API-call budget it
/// asserts is vacuous.
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

/// The `adapter_apps` row every Telegram Bot API call resolves its client from.
///
/// The bot token is a secret, so this goes through
/// `adapter_apps::upsert_adapter_app`; without it the ingress path turns a
/// missing app into a `400` and refuses every delivery.
#[allow(dead_code)]
pub async fn seed_telegram_app(pool: &sqlx::PgPool, config: &Config, bot_token: &str) {
    moatless_identity::adapter_apps::upsert_adapter_app(
        pool,
        &config.secrets,
        moatless_identity::adapter_apps::AdapterAppInput {
            adapter_kind: "telegram",
            app_key: moatless_identity::adapter_apps::DEFAULT_APP_KEY.to_string(),
            display_name: "Telegram (test)".to_string(),
            config_json: serde_json::json!({ "mode": "webhook" }),
            secrets: vec![moatless_identity::adapter_apps::AdapterAppSecretInput {
                name: "bot_token",
                plaintext: bot_token.to_string(),
            }],
        },
        USER_1,
    )
    .await
    .expect("seed Telegram adapter app");
}

/// Exclusive, restoring ownership of `GITHUB_API_BASE_URL` for the caller.
///
/// The GitHub REST base is resolved from that env var on every client build, so
/// pointing the backend at a stub writes a process-global; the lock lives here,
/// not per test file, so one file's stub cannot answer another's calls in a
/// one-process `cargo test` run.
#[allow(dead_code)]
pub async fn github_api_base(base_url: &str) -> GithubApiBaseGuard {
    static LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    let guard = LOCK.lock().await;
    let previous = std::env::var_os("GITHUB_API_BASE_URL");
    std::env::set_var("GITHUB_API_BASE_URL", base_url);
    GithubApiBaseGuard {
        _lock: guard,
        previous,
    }
}

#[allow(dead_code)]
pub struct GithubApiBaseGuard {
    _lock: tokio::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}

impl Drop for GithubApiBaseGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            std::env::set_var("GITHUB_API_BASE_URL", value);
        } else {
            std::env::remove_var("GITHUB_API_BASE_URL");
        }
    }
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
            workspace_repository_secret: None,
            workspace_repository_role_arn: None,
            compute_mode: moatless_core::config::KubernetesComputeMode::Standard,
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
            persist_toolchain: false,
            storage_enabled: false,
            storage_size: "1Gi".into(),
            storage_class: None,
            default_pvc_enabled: true,
            default_s3_enabled: true,
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
            idle_check_interval_secs: 300,
            inactive_task_timeout_days: 0,
            cold_after_hours: 0,
            orphan_gc_enabled: false,
            codex_refresher_interval_secs: 0,
            max_crash_restart_count: 5,
            sandbox_resources_cpu_request: "1".into(),
            sandbox_resources_cpu_limit: "4".into(),
            sandbox_resources_memory_request: "2Gi".into(),
            sandbox_resources_memory_limit: "8Gi".into(),
            ephemeral_storage: None,
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
            github_required_org: None,
            auth0_domain: None,
            auth0_client_id: None,
            auth0_client_secret: None,
            auth0_callback_url: None,
            auth0_audience: None,
            auth0_groups_claim: String::new(),
            oidc: None,
            session_max_age_secs: 3600,
            session_absolute_age_secs: 3600,
            frontend_url: "http://localhost:3000".into(),
            csrf_trusted_origins: Vec::new(),
            cookie_domain: None,
            session_cookie_name: "test_session".into(),
            is_production: false,
            admin_username: "admin".into(),
            admin_email: Some("admin@example.com".into()),
            admin_password: None,
            break_glass_enabled: false,
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
        otel: OtelConfig {
            otlp_endpoint: None,
            sampling_ratio: 0.1,
            service_name: "test".into(),
        },
    }
}

/// Build an `AppState` with a real `PgPool` and mock sandbox capabilities.
#[allow(dead_code)]
pub fn test_app_state(pool: PgPool) -> AppState {
    test_app_state_with_config(pool, test_config(), false)
}

/// Build an `AppState` with every external Adapter installed through the same
/// seam production startup uses.
#[allow(
    dead_code,
    reason = "this module is compiled into every test binary; each one uses a subset of it"
)]
pub fn test_app_state_with_adapters(pool: PgPool, config: Config) -> AppState {
    test_app_state_with_config(pool, config, true)
}

/// Build an `AppState` with the supplied Adapter configuration.
#[allow(
    dead_code,
    reason = "this module is compiled into every test binary; each one uses a subset of it"
)]
pub fn test_app_state_with_linear(pool: PgPool, config: Config) -> AppState {
    test_app_state_with_adapters(pool, config)
}

/// [`test_app_state_with_linear`] whose event stream is a recorder.
///
/// The returned publisher is the same `Arc` the state holds, so a test reads
/// back the actor and payload of every task event the workflow published.
#[allow(
    dead_code,
    reason = "this module is compiled into every test binary; each one uses a subset of it"
)]
pub fn test_app_state_with_recorded_events(
    pool: PgPool,
    config: Config,
) -> (AppState, Arc<RecordingPublisher>) {
    let recorder = Arc::new(RecordingPublisher::new());
    let state = build_app_state(pool, config, true, Some(recorder.clone()));
    (state, recorder)
}

/// [`test_app_state_with_adapters`] with the config chosen by the caller.
///
/// The seams deciding which host an outbound call reaches live on `Config` —
/// `linear.api_base_url` is the one this exists for — so a workflow building
/// its API client from `state.config` can be pointed at a stub no other way.
#[allow(
    dead_code,
    reason = "this module is compiled into every test binary; each one uses a subset of it"
)]
pub fn test_app_state_with_config(
    pool: PgPool,
    config: Config,
    install_adapters: bool,
) -> AppState {
    build_app_state(pool, config, install_adapters, None)
}

fn build_app_state(
    pool: PgPool,
    config: Config,
    install_adapters: bool,
    events: Option<Arc<RecordingPublisher>>,
) -> AppState {
    let ports = compose(pool.clone(), &config, None);
    let adapter_installations = install_adapters.then(|| {
        Arc::new(
            moatless_backend::adapter_installations::AdapterInstallations::install(&config, &ports),
        )
    });
    let config = Arc::new(config);
    let connector: Arc<dyn SandboxConnector> = Arc::new(MockSandboxConnector);
    let sandbox_graph = ports.sandbox_graph();
    let lifecycle = sandbox_graph.lifecycle(LifecycleInputs {
        lock_pool: pool.clone(),
        runtime: Arc::new(MockSandboxRuntime),
        connector: connector.clone(),
        plan: sandbox_graph.provision_deps(
            ports.workspace_resolver.clone(),
            ports.user_directory.clone(),
        ),
        default_sandbox_image: "mock-sandbox:latest".into(),
        // No Redis and no S3 in the handler tests, so no publication and no
        // cold transition.
        events: None,
        workspace_archive: None,
    });
    let events: Option<Arc<dyn moatless_core::events::EventPublisher>> =
        events.map(|r| r as Arc<dyn moatless_core::events::EventPublisher>);
    AppState::new(
        &ports,
        config,
        lifecycle,
        connector,
        Arc::new(MockNotify),
        events,
        None, // s3
        adapter_installations,
    )
}

// =========================================================================
// Factory functions
// =========================================================================

/// Well-known test user ids.
///
/// `AuthIdentity::user_id` is a `Uuid`, so HTTP handler tests authenticate as a
/// real uuid and seed the matching row's owner with the same value; DB-only DAO
/// tests keep symbolic names (`"user-1"`) since they build `ScopeContext`
/// directly and never parse a uuid.
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

// =========================================================================
// Adapter delivery
// =========================================================================

/// `worker_state` is deliberately the caller's *second* replica.
///
/// A delivery is answered once durably owned, so nothing has reached the Agent
/// when the route returns; what executes a record never depends on the process
/// that accepted it.
#[allow(
    dead_code,
    reason = "this module is compiled into every integration test binary and each uses a \
              subset of it, so `expect` would fire in the binaries that do call this"
)]
pub async fn drain_linear_adapter_delivery(pool: &PgPool, worker_state: AppState) {
    let mut handlers = moatless_adapter_model::delivery::AdapterDeliveryHandlers::new();
    let router = Arc::new(
        moatless_backend::adapters::providers::linear::delivery::LinearInboundRouter::new(
            worker_state.connection_routing.clone(),
            worker_state.harness_credentials.clone(),
            worker_state.config.clone(),
            worker_state.adapter_apps.clone(),
            worker_state.loop_reader.clone(),
            worker_state.task_bindings.clone(),
            worker_state.task_reader.clone(),
            worker_state.task_creation.clone(),
            worker_state.relation_recorder.clone(),
            worker_state.message_send.clone(),
            worker_state.sandbox_waker.clone(),
            worker_state.task_closer.clone(),
            // The gate is process-local and only the event-handler consumer
            // reads it; no consumer runs here, so this replica gets its own.
            Arc::new(
                moatless_backend::adapters::providers::linear::active_turns::ActiveLinearTurns::new(
                ),
            ),
            worker_state.routing.clone(),
        ),
    );
    moatless_backend::adapters::providers::linear::delivery::register(&mut handlers, router);

    let cancel = tokio_util::sync::CancellationToken::new();
    let handle = worker_state
        .adapter_delivery_worker
        .spawn(Arc::new(handlers), cancel.clone());

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
        .insert(moatless_types::auth::AuthIdentity {
            user_id: Uuid::parse_str(user_id)
                .expect("authed_request user_id must be a UUID (use common::USER_1)"),
            login: user_id.to_string(),
            role: moatless_types::user::UserRole::User,
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
        .insert(moatless_types::auth::AuthIdentity {
            user_id: Uuid::parse_str(user_id)
                .expect("authed_request user_id must be a UUID (use common::USER_1)"),
            login: user_id.to_string(),
            role: moatless_types::user::UserRole::User,
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

/// The row factories the workspaces band owns, so a fixture here writes the
/// same shape that band's own PostgreSQL tests write.
#[allow(unused_imports)]
pub use moatless_workspaces::test_support::{
    make_repository_row, make_workspace_repo_row, make_workspace_row, run_config, seed_team,
};

/// The row factories the tasks band owns, so a fixture here writes the same
/// shape that band's own PostgreSQL tests write.
#[allow(unused_imports)]
pub use moatless_tasks::message::test_support::make_message_row;
#[allow(unused_imports)]
pub use moatless_tasks::task::test_support::{FIXTURE_WORKSPACE, make_task_row};
