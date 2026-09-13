use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::Instant;

use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_client::EnvironmentManager;
use codex_app_server_client::InProcessAppServerClient;
use codex_app_server_client::InProcessClientStartArgs;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigWarningNotification;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadResumeParams;
use codex_app_server_protocol::ThreadResumeResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_arg0::Arg0DispatchPaths;
use codex_cloud_requirements::cloud_requirements_loader_for_storage;
use codex_config::LoaderOverrides;
use codex_core::config::Config;
use codex_core::config::ConfigBuilder;
use codex_core::config::ConfigOverrides;
use codex_core::config::find_codex_home;
use codex_core::config::load_config_as_toml_with_cli_overrides;
use codex_feedback::CodexFeedback;
use codex_protocol::protocol::SessionSource;
use codex_utils_absolute_path::AbsolutePathBuf;

use crate::error::{Result, SdkError};
use crate::options::CodexOptions;

/// Holds the initialized runtime state.
pub(crate) struct RuntimeState {
    pub client: InProcessAppServerClient,
    pub thread_id: String,
    pub _config: Config,
    pub worker: RuntimeWorker,
}

/// Owns the Tokio runtime that contains the upstream app-server tasks.
pub(crate) struct RuntimeWorker {
    stop: oneshot::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl RuntimeWorker {
    /// Stops the runtime and joins its owner thread.
    pub async fn shutdown(self) {
        let _ = self.stop.send(());
        let _ = tokio::task::spawn_blocking(move || self.thread.join()).await;
    }
}

/// Boots the in-process app-server runtime from SDK options and creates an
/// initial thread.
pub(crate) async fn start_runtime(options: &CodexOptions) -> Result<RuntimeState> {
    let options = options.clone();
    let timeout = options.init_timeout.unwrap_or(Duration::from_secs(60));
    let ((client, thread_id, config), worker) =
        isolated_runtime(timeout, async move { initialize_runtime(&options).await }).await?;
    Ok(RuntimeState {
        client,
        thread_id,
        _config: config,
        worker,
    })
}

async fn isolated_runtime<T: Send + 'static>(
    timeout: Duration,
    initialize: impl Future<Output = Result<T>> + Send + 'static,
) -> Result<(T, RuntimeWorker)> {
    let deadline = Instant::now() + timeout;
    let (mut ready_tx, ready_rx) = oneshot::channel();
    let (stop, stop_rx) = oneshot::channel();
    // Upstream startup hides its spawned tasks until Initialize completes.
    // A separate runtime lets the deadline terminate them before returning.
    let thread = std::thread::Builder::new()
        .name("codex-sdk-runtime".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(8 * 1024 * 1024)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(SdkError::Startup(error)));
                    return;
                }
            };
            runtime.block_on(async {
                let result = tokio::select! {
                    biased;
                    _ = ready_tx.closed() => return,
                    _ = tokio::time::sleep_until(deadline) => Err(initialization_timeout()),
                    result = initialize => result,
                };
                let succeeded = result.is_ok();
                if ready_tx.send(result).is_ok() && succeeded {
                    let _ = stop_rx.await;
                }
            });
            runtime.shutdown_timeout(Duration::from_secs(5));
        })
        .map_err(SdkError::Startup)?;
    let worker = RuntimeWorker { stop, thread };
    match ready_rx.await {
        Ok(Ok(state)) => Ok((state, worker)),
        result => {
            worker.shutdown().await;
            Err(match result {
                Ok(Err(error)) => error,
                _ => SdkError::Internal("runtime worker stopped during initialization".into()),
            })
        }
    }
}

async fn initialize_runtime(
    options: &CodexOptions,
) -> Result<(InProcessAppServerClient, String, Config)> {
    let (start_args, config) = configure_runtime(options).await?;
    let client = InProcessAppServerClient::start(start_args)
        .await
        .map_err(SdkError::Startup)?;
    match initialize_thread(&client, options).await {
        Ok(thread_id) => Ok((client, thread_id, config)),
        Err(error) => {
            let _ = client.shutdown().await;
            Err(error)
        }
    }
}

fn initialization_timeout() -> SdkError {
    SdkError::Startup(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "runtime initialization timed out",
    ))
}

async fn configure_runtime(options: &CodexOptions) -> Result<(InProcessClientStartArgs, Config)> {
    let codex_home = find_codex_home().map_err(|e| SdkError::Config(format!("{e}")))?;

    let cwd = match &options.cwd {
        Some(p) => AbsolutePathBuf::from_absolute_path(
            p.canonicalize()
                .map_err(|e| SdkError::Config(format!("bad cwd: {e}")))?,
        )
        .map_err(|e| SdkError::Config(format!("bad cwd: {e}")))?,
        None => AbsolutePathBuf::current_dir()
            .map_err(|e| SdkError::Config(format!("cannot determine cwd: {e}")))?,
    };

    let config_toml = load_config_as_toml_with_cli_overrides(
        &codex_home,
        Some(&cwd),
        options.config_overrides.clone(),
        LoaderOverrides::default(),
    )
    .await
    .map_err(|e| SdkError::Config(format!("config load error: {e}")))?;

    let chatgpt_base_url = config_toml
        .chatgpt_base_url
        .clone()
        .unwrap_or_else(|| "https://chatgpt.com/backend-api/".to_string());
    let cloud_requirements = cloud_requirements_loader_for_storage(
        codex_home.to_path_buf(),
        true,
        config_toml.cli_auth_credentials_store.unwrap_or_default(),
        chatgpt_base_url,
    )
    .await;

    // Convert v2 types to core types for ConfigOverrides.
    let core_approval_policy = options
        .approval_policy
        .map(codex_app_server_protocol::AskForApproval::to_core)
        .or(Some(codex_protocol::protocol::AskForApproval::OnRequest));
    let core_sandbox_mode = options
        .sandbox_mode
        .map(codex_app_server_protocol::SandboxMode::to_core);

    let overrides = ConfigOverrides {
        model: options.model.clone(),
        cwd: options.cwd.clone(),
        approval_policy: core_approval_policy,
        sandbox_mode: core_sandbox_mode,
        model_provider: options.model_provider.clone(),
        base_instructions: options.base_instructions.clone(),
        developer_instructions: options.developer_instructions.clone(),
        ephemeral: options.ephemeral,
        ..Default::default()
    };

    let config = ConfigBuilder::default()
        .cli_overrides(options.config_overrides.clone())
        .harness_overrides(overrides)
        .cloud_requirements(cloud_requirements.clone())
        .build()
        .await
        .map_err(|e| SdkError::Config(format!("{e}")))?;

    let config_warnings: Vec<ConfigWarningNotification> = config
        .startup_warnings
        .iter()
        .map(|warning| ConfigWarningNotification {
            summary: warning.clone(),
            details: None,
            path: None,
            range: None,
        })
        .collect();

    let arg0_paths = Arg0DispatchPaths {
        codex_self_exe: None,
        codex_linux_sandbox_exe: None,
        main_execve_wrapper_exe: None,
    };

    let start_args = InProcessClientStartArgs {
        arg0_paths,
        config: Arc::new(config.clone()),
        cli_overrides: options.config_overrides.clone(),
        loader_overrides: LoaderOverrides::default(),
        strict_config: false,
        cloud_requirements,
        feedback: CodexFeedback::new(),
        log_db: None,
        state_db: None,
        environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
        config_warnings,
        session_source: SessionSource::Custom("sdk".to_string()),
        enable_codex_api_key_env: true,
        client_name: "codex_sdk".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        experimental_api: true,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    };

    Ok((start_args, config))
}

async fn initialize_thread(
    client: &InProcessAppServerClient,
    options: &CodexOptions,
) -> Result<String> {
    let thread_id = if let Some(ref thread_id) = options.resume_thread_id {
        // Resume an existing thread.
        let thread_resume_response: ThreadResumeResponse = client
            .request_typed(ClientRequest::ThreadResume {
                request_id: RequestId::Integer(1),
                params: ThreadResumeParams {
                    thread_id: thread_id.clone(),
                    model: options.model.clone(),
                    model_provider: options.model_provider.clone(),
                    cwd: options.cwd.as_ref().map(|p| p.display().to_string()),
                    approval_policy: options.approval_policy,
                    sandbox: options.sandbox_mode,
                    base_instructions: options.base_instructions.clone(),
                    developer_instructions: options.developer_instructions.clone(),
                    ..Default::default()
                },
            })
            .await
            .map_err(|e| SdkError::Request(format!("thread/resume failed: {e}")))?;

        thread_resume_response.thread.id
    } else {
        // Create a new thread.
        let thread_start_response: ThreadStartResponse = client
            .request_typed(ClientRequest::ThreadStart {
                request_id: RequestId::Integer(1),
                params: ThreadStartParams {
                    model: options.model.clone(),
                    model_provider: options.model_provider.clone(),
                    base_instructions: options.base_instructions.clone(),
                    developer_instructions: options.developer_instructions.clone(),
                    ephemeral: options.ephemeral,
                    cwd: options.cwd.as_ref().map(|p| p.display().to_string()),
                    approval_policy: options.approval_policy,
                    sandbox: options.sandbox_mode,
                    ..Default::default()
                },
            })
            .await
            .map_err(|e| SdkError::Request(format!("thread/start failed: {e}")))?;

        thread_start_response.thread.id
    };

    Ok(thread_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::pending;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct TaskLifetime(Arc<AtomicBool>);

    impl Drop for TaskLifetime {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn initialization_timeout_destroys_started_tasks_before_returning() {
        let stopped = Arc::new(AtomicBool::new(false));
        let lifetime = TaskLifetime(stopped.clone());
        let result = isolated_runtime(Duration::from_millis(20), async move {
            tokio::spawn(async move {
                let _lifetime = lifetime;
                pending::<()>().await;
            });
            pending::<Result<()>>().await
        })
        .await;
        assert!(
            matches!(result, Err(SdkError::Startup(error)) if error.kind() == std::io::ErrorKind::TimedOut)
        );
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn successful_initialization_keeps_tasks_until_owned_shutdown() {
        let stopped = Arc::new(AtomicBool::new(false));
        let lifetime = TaskLifetime(stopped.clone());
        let ((), worker) = isolated_runtime(Duration::from_secs(60), async move {
            tokio::spawn(async move {
                let _lifetime = lifetime;
                pending::<()>().await;
            });
            Ok(())
        })
        .await
        .unwrap();
        assert!(!stopped.load(Ordering::SeqCst));
        worker.shutdown().await;
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn initialization_failure_destroys_started_tasks() {
        let stopped = Arc::new(AtomicBool::new(false));
        let lifetime = TaskLifetime(stopped.clone());
        let result = isolated_runtime(Duration::from_secs(60), async move {
            tokio::spawn(async move {
                let _lifetime = lifetime;
                pending::<()>().await;
            });
            Err::<(), _>(SdkError::Request("thread start failed".into()))
        })
        .await;
        assert!(matches!(result, Err(SdkError::Request(_))));
        assert!(stopped.load(Ordering::SeqCst));
    }
}
