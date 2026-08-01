//! Periodic credential refresher.
//!
//! Background tokio task that re-runs `fetch_and_apply_credentials` every
//! `CREDENTIAL_REFRESH_INTERVAL_SECS` seconds (default 900 = 15 min). Keeps
//! rotated GitHub App tokens (1h TTL) and GitHub user access tokens (8h TTL)
//! fresh without requiring a task restart.
//!
//! Each cycle:
//! - Updates sandbox process env vars (future-spawned subprocesses pick up the
//!   refreshed token).
//! - Refreshes `state.credentials` (next agent run gets fresh values).
//! - Rewrites credential files (`.credentials`, `auth.json`, `server.env`,
//!   `~/.config/gh/hosts.yml`).

use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::credentials::{self, PersistMode};
use crate::state::SandboxState;

/// Default refresh interval when the env var isn't set.
const DEFAULT_INTERVAL_SECS: u64 = 900;

/// Read the refresh interval from `CREDENTIAL_REFRESH_INTERVAL_SECS` (default 900s).
pub fn read_interval_from_env() -> Duration {
    let secs = std::env::var("CREDENTIAL_REFRESH_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_INTERVAL_SECS);
    Duration::from_secs(secs)
}

/// Periodically refresh credentials until cancelled.
pub async fn run_credential_refresher(
    state: SandboxState,
    cancel: CancellationToken,
    interval: Duration,
) {
    info!(
        task_id = %state.config.task_id,
        interval_secs = interval.as_secs(),
        "Credential refresher started"
    );

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!(
                    task_id = %state.config.task_id,
                    "Credential refresher shutting down"
                );
                return;
            }
            () = tokio::time::sleep(interval) => {
                refresh_once(&state, interval).await;
            }
        }
    }
}

async fn refresh_once(state: &SandboxState, interval: Duration) {
    match credentials::fetch_and_apply_credentials_with_mode(state, PersistMode::Refresh).await {
        Ok(()) => {
            let count = state.credentials.read().await.len();
            info!(
                task_id = %state.config.task_id,
                credential_count = count,
                interval_secs = interval.as_secs(),
                "Credentials refreshed"
            );
        }
        Err(e) => {
            // Swallow errors so transient backend blips don't kill the loop.
            warn!(
                task_id = %state.config.task_id,
                error = %e,
                "Credential refresh failed, will retry on next interval"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use sandbox_proto::{GetCredentialsResponse, PendingAgentDelivery, PublishEventRequest};
    use tokio::sync::RwLock;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::agent::agent_state::AgentState;
    use crate::agent::handle::spawn_agent;
    use crate::backend_client::BackendClient;
    use crate::config::{AgentConfig, Config, ProxyConfig};
    use crate::credentials::Dispatcher;
    use crate::credentials::recovery::CredentialFailureDetector;
    use crate::events::GrpcEventPublisher;
    use crate::init::InitPhase;
    use crate::state::SandboxState;

    struct CountingBackend {
        call_count: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl BackendClient for CountingBackend {
        async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), tonic::Status> {
            Ok(())
        }

        async fn get_pending_messages(
            &self,
            _task_id: &str,
        ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
            Ok(vec![])
        }

        async fn acknowledge_messages(
            &self,
            _task_id: &str,
            _message_uuids: Vec<String>,
            _session_id: &str,
            _run_id: &str,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }

        async fn get_credentials(
            &self,
            _task_id: &str,
        ) -> Result<GetCredentialsResponse, tonic::Status> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(GetCredentialsResponse {
                credentials: HashMap::new(),
                server_env: HashMap::new(),
                ssh_keys: HashMap::new(),
            })
        }

        async fn update_credential(
            &self,
            _task_id: &str,
            _key: &str,
            _value: &str,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }

        async fn get_system_prompt(&self, _task_id: &str) -> Result<String, tonic::Status> {
            Ok(String::new())
        }

        async fn download_upload(
            &self,
            _task_id: &str,
            _file_id: &str,
        ) -> Result<sandbox_proto::DownloadUploadResponse, tonic::Status> {
            Err(tonic::Status::unimplemented("not used"))
        }

        async fn recover_interrupted_run(
            &self,
            _task_id: &str,
        ) -> Result<Option<sandbox_proto::InterruptedTurn>, tonic::Status> {
            Ok(None)
        }

        async fn upload_transcript(
            &self,
            _task_id: &str,
            _tar_data: Vec<u8>,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }

        async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, tonic::Status> {
            Ok(Vec::new())
        }
    }

    fn test_config(tmp: &tempfile::TempDir) -> Config {
        let home = tmp.path().join("home");
        Config {
            port: 0,
            grpc_port: 0,
            task_id: "task-refresher".to_string(),
            agent_type: None,
            workspace_dir: tmp.path().join("workspace").display().to_string(),
            repo_dir: tmp.path().join("workspace").display().to_string(),
            data_dir: tmp.path().join("data").display().to_string(),
            upload_dir: tmp.path().join("uploads").display().to_string(),
            log_level: "info".to_string(),
            backend_grpc_url: "http://localhost:50051".to_string(),
            backend_api_url: None,
            user_auth_token: None,
            bootstrap_token: None,
            agent_config: AgentConfig::default(),
            claude_cli_path: "/usr/bin/claude".to_string(),
            codex_cli_path: "/usr/bin/codex".to_string(),
            proxy: ProxyConfig::default(),
            servers: vec![],
            frontend_url: None,
            use_tmux: false,
            background_job_monitoring_enabled: false,
            permission_mode: claude_agent_sdk::PermissionMode::BypassPermissions,
            workspace_sync_enabled: false,
            default_model: None,
            default_codex_model: None,
            home_dir: home.display().to_string(),
            codex_home_dir: home.join(".codex").display().to_string(),
            claude_config_dir: home.join(".claude").display().to_string(),
            git_clone_url: None,
            git_branch: None,
            git_provider: "github".to_string(),
            repos: Vec::new(),
            repo_config_raw: None,
            setup_commands: vec![],
            plugins: vec![],
        }
    }

    fn test_state(tmp: &tempfile::TempDir, backend: Arc<dyn BackendClient>) -> SandboxState {
        use crate::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};

        let (outbox, _drainer) = crate::outbox::EventOutbox::spawn(
            Arc::clone(&backend),
            "test-task".to_string(),
            tokio_util::sync::CancellationToken::new(),
        );
        let events = GrpcEventPublisher::new(outbox);
        let config = Arc::new(test_config(tmp));
        let agent_state = Arc::new(AgentState::new());
        let credentials = Arc::new(RwLock::new(HashMap::new()));

        let late_injector = Arc::new(LateInjector::new());
        let scheduled_jobs = Arc::new(ScheduledJobRegistry::new(Arc::clone(&late_injector) as _));

        let credential_failures = CredentialFailureDetector::default();
        let credential_dispatcher = Arc::new(Dispatcher::new());
        let system_prompt_cache: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let heartbeat = crate::agent::heartbeat::Heartbeat::new();

        let agent_handle = spawn_agent(
            Arc::clone(&config),
            Arc::clone(&agent_state),
            events.clone(),
            Arc::clone(&credentials),
            Arc::clone(&backend),
            Arc::clone(&scheduled_jobs),
            credential_failures.clone(),
            Arc::clone(&credential_dispatcher),
            Arc::clone(&system_prompt_cache),
            heartbeat.clone(),
        );
        late_injector.set(Box::new(agent_handle.clone()));

        SandboxState {
            config,
            events,
            agent: agent_state,
            agent_handle,
            heartbeat,
            scheduled_jobs,
            backend_client: backend,
            init_phase: Arc::new(RwLock::new(InitPhase::Ready)),
            credentials,
            lifecycle_cancel: CancellationToken::new(),
            credential_dispatcher,
            credential_failures,
            codex_watcher_handle: Arc::new(RwLock::new(None)),
            system_prompt_cache,
            dispatch_lock: Default::default(),
            supervisor_health: Default::default(),
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn refresher_fetches_repeatedly_and_stops_on_cancel() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(tmp.path().join("home/.claude"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(tmp.path().join("home/.codex"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(tmp.path().join("data"))
            .await
            .unwrap();

        let backend = Arc::new(CountingBackend {
            call_count: AtomicUsize::new(0),
        });
        let state = test_state(&tmp, Arc::clone(&backend) as _);

        let cancel = state.lifecycle_cancel.clone();
        let refresher_state = state.clone();
        let handle = tokio::spawn(run_credential_refresher(
            refresher_state,
            cancel.clone(),
            Duration::from_millis(50),
        ));

        // Drive virtual time forward past several refresh intervals.
        // With `start_paused`, tokio skips to the next scheduled wake-up rather
        // than sleeping in real time — this is fast and deterministic.
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(60)).await;
        }

        cancel.cancel();
        handle.await.unwrap();

        let calls = backend.call_count.load(Ordering::SeqCst);
        assert!(calls >= 3, "expected at least 3 refresh calls, got {calls}");
    }
}
