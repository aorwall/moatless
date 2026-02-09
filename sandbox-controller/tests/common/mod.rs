//! Shared test helpers for integration tests.

use std::sync::Arc;

use async_trait::async_trait;
use axum::response::sse::Event;
use futures::stream::BoxStream;
use sqlx::PgPool;

use sandbox_controller::config::Config;
use sandbox_controller::db::models::*;
use sandbox_controller::sandbox::error::SandboxError;
use sandbox_controller::sandbox::provider::SandboxProvider;
use sandbox_controller::sandbox::types::*;
use sandbox_controller::state::AppState;

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

    async fn status(&self, _task_id: &str) -> Result<StatusResponse, SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn health(&self, _task_id: &str) -> HealthResponse {
        HealthResponse {
            healthy: false,
        }
    }

    async fn workspace_status(
        &self,
        _task_id: &str,
    ) -> Result<WorkspaceStatusFullResponse, SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn stop(&self, _task_id: &str) -> Result<(), SandboxError> {
        Ok(()) // best-effort in handlers
    }

    async fn start(&self, _task_id: &str) -> Result<(), SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn restart(&self, _task_id: &str) -> Result<(), SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn cleanup(&self, _task_id: &str) -> Result<(), SandboxError> {
        Ok(()) // best-effort in handlers
    }

    async fn server_status(
        &self,
        _task_id: &str,
    ) -> Result<ServerStatusResponse, SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn server_logs(
        &self,
        _task_id: &str,
        _server_name: &str,
    ) -> Result<BoxStream<'static, Result<Event, SandboxError>>, SandboxError> {
        Err(SandboxError::Internal("mock: not implemented".into()))
    }

    async fn search_files(
        &self,
        _task_id: &str,
        _query: &str,
        _limit: u32,
    ) -> Result<Vec<FileSearchResult>, SandboxError> {
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
        port: 0,
        metrics_port: 0,
        sandbox_namespace: "test".into(),
        sandbox_pod_template_path: None,
        k8s_sandbox_storage_enabled: false,
        k8s_sandbox_storage_size: "1Gi".into(),
        k8s_sandbox_storage_class: None,
        k8s_pod_startup_timeout_secs: 10,
        task_token_secret: None,
        proxy_domain: None,
        proxy_protocol: "http".into(),
        traefik_auth_middleware_name: "test-auth".into(),
        traefik_auth_middleware_namespace: "test".into(),
        traefik_entry_point: "web".into(),
        database_url: None,
        redis_url: None,
        redis_max_stream_length: 100,
        sse_broadcast_capacity: 16,
        sse_consumer_id: "test".into(),
        auth_provider: None,
        github_client_id: None,
        github_client_secret: None,
        github_callback_url: None,
        github_oauth_scope: "".into(),
        auth0_domain: None,
        auth0_client_id: None,
        auth0_client_secret: None,
        auth0_callback_url: None,
        auth0_audience: None,
        auth0_groups_claim: "".into(),
        session_max_age_secs: 3600,
        frontend_url: "http://localhost:3000".into(),
        cookie_domain: None,
        session_cookie_name: "test_session".into(),
        is_production: false,
        auth_cache_ttl_secs: 0,
        s3_bucket_name: None,
        s3_key_prefix: "test".into(),
        s3_region: "us-east-1".into(),
        sandbox_file_mount_path: "/tmp/test".into(),
    }
}

/// Build an AppState with a real PgPool and mock sandbox provider.
#[allow(dead_code)]
pub fn test_app_state(pool: PgPool) -> AppState {
    AppState {
        db: pool,
        config: Arc::new(test_config()),
        sandbox: Arc::new(MockSandboxProvider),
        events: None,
        s3: None,
    }
}

// =========================================================================
// Factory functions
// =========================================================================

/// Create a minimal TaskRow for tests.
#[allow(dead_code)]
pub fn make_task_row(task_id: &str, user_id: &str) -> TaskRow {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    TaskRow {
        task_id: task_id.to_string(),
        short_id: Some(format!("s-{task_id}")),
        created_at: now.clone(),
        last_message_at: now,
        agent_type: "claude-code".to_string(),
        repository_id: "repo-1".to_string(),
        status: "open".to_string(),
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
        visibility: "private".to_string(),
        created_at_db: None,
    }
}

/// Create a minimal RepositoryRow for tests.
#[allow(dead_code)]
pub fn make_repository_row(id: &str, user_id: &str) -> RepositoryRow {
    let now = chrono::Utc::now();
    RepositoryRow {
        id: id.to_string(),
        name: format!("repo-{id}"),
        kind: "git".to_string(),
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
        provider: Some("github".to_string()),
        visibility: Some("public".to_string()),
        created_by: Some(user_id.to_string()),
        deleted: Some(false),
        created_at: Some(now),
        updated_at: Some(now),
    }
}

/// Create a minimal TaskMessageRow for tests.
#[allow(dead_code)]
pub fn make_message_row(task_id: &str, uuid: &str) -> TaskMessageRow {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    TaskMessageRow {
        id: 0, // ignored on insert (serial)
        task_id: task_id.to_string(),
        uuid: uuid.to_string(),
        message_type: "user".to_string(),
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
        message_role: "user".to_string(),
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
        attachments: None,
        agent_session_id: None,
        turn_status: None,
        turn_result: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode: None,
        model: None,
        resume_session_id: None,
        sent_at: None,
        created_at: None,
    }
}

/// Create a minimal CommandRow for tests.
#[allow(dead_code)]
pub fn make_command_row(name: &str) -> CommandRow {
    CommandRow {
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

/// Build an HTTP request with auth headers.
#[allow(dead_code)]
pub fn authed_request(method: &str, uri: &str, user_id: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("x-user-id", user_id)
        .header("x-user-login", user_id)
        .header("content-type", "application/json")
        .body(axum::body::Body::empty())
        .unwrap()
}

/// Build an HTTP request with auth headers and a JSON body.
#[allow(dead_code)]
pub fn authed_json_request(
    method: &str,
    uri: &str,
    user_id: &str,
    body: serde_json::Value,
) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("x-user-id", user_id)
        .header("x-user-login", user_id)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
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
