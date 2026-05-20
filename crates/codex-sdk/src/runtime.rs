use std::sync::Arc;

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
}

/// Boots the in-process app-server runtime from SDK options and creates
/// an initial thread.
pub(crate) async fn start_runtime(options: &CodexOptions) -> Result<RuntimeState> {
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
        cloud_requirements,
        feedback: CodexFeedback::new(),
        log_db: None,
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

    let client = InProcessAppServerClient::start(start_args)
        .await
        .map_err(SdkError::Startup)?;

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

    Ok(RuntimeState {
        client,
        thread_id,
        _config: config,
    })
}
