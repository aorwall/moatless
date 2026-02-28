//! Configuration loaded from environment variables.
//!
//! The sandbox runs inside a sandbox pod and receives its configuration
//! via environment variables set by the orchestrator.

use std::env;

use serde::Deserialize;

/// Agent-specific configuration parsed from REPO_CONFIG.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct AgentConfig {
    pub plugins: Vec<String>,
    pub plugin_paths: Vec<String>,
    pub timeout: Option<u64>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
}

/// Code execution configuration from REPO_CONFIG.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CodeExecutionConfig {
    #[serde(default)]
    pub setup_commands: Vec<String>,
}

/// Partial REPO_CONFIG JSON structure — we only parse the fields we need.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoConfigFile {
    #[serde(default)]
    agent_config: Option<RepoAgentConfig>,
    #[serde(default)]
    code_execution: Option<CodeExecutionConfig>,
    #[serde(default)]
    plugin_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoAgentConfig {
    #[serde(default)]
    plugins: Vec<String>,
    #[serde(default)]
    plugin_paths: Vec<String>,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    system_prompt: Option<String>,
}

/// Parsed REPO_CONFIG result.
struct ParsedRepoConfig {
    agent_config: AgentConfig,
    setup_commands: Vec<String>,
    plugin_ids: Vec<String>,
}

/// Main sandbox configuration.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    /// HTTP port (default: 8001) — kept for K8s health probes
    pub port: u16,
    /// gRPC port for sandbox service (default: 50052)
    pub grpc_port: u16,
    /// Task ID (required)
    pub task_id: String,
    /// Workspace directory
    pub workspace_dir: String,
    /// Data directory for server artifacts
    pub data_dir: String,
    /// Upload directory
    pub upload_dir: String,
    /// Log level
    pub log_level: String,
    /// Backend gRPC URL (e.g. "http://backend:50051") — required
    pub backend_grpc_url: String,
    /// Backend API URL for legacy HTTP calls (kept for transition)
    pub backend_api_url: Option<String>,
    /// User auth token for backend API calls
    pub user_auth_token: Option<String>,
    /// Agent-specific configuration
    pub agent_config: AgentConfig,
    /// Path to the Claude CLI binary
    pub claude_cli_path: String,

    // ── Init-related fields ────────────────────────────────────────
    /// Home directory (default: "/home/moatless")
    pub home_dir: String,
    /// Claude config directory (default: "$HOME/.claude")
    pub claude_config_dir: String,
    /// Git clone URL (from pod env)
    pub git_clone_url: Option<String>,
    /// Git branch to clone
    pub git_branch: Option<String>,
    /// Git provider name (github, gitlab, gitness)
    pub git_provider: String,
    /// Raw REPO_CONFIG JSON (for entrypoint compat)
    pub repo_config_raw: Option<String>,
    /// Setup commands from REPO_CONFIG.codeExecution.setupCommands
    pub setup_commands: Vec<String>,
    /// Plugin IDs from REPO_CONFIG.pluginIds
    pub plugin_ids: Vec<String>,
}

impl Config {
    /// Load configuration from environment variables.
    pub fn from_env() -> Self {
        let parsed = parse_repo_config();

        let home_dir = env::var("HOME")
            .unwrap_or_else(|_| "/home/moatless".to_string());

        let claude_config_dir = env::var("CLAUDE_CONFIG_DIR")
            .unwrap_or_else(|_| format!("{home_dir}/.claude"));

        Self {
            port: env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8001),

            grpc_port: env::var("GRPC_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50052),

            task_id: env::var("TASK_ID").unwrap_or_default(),

            workspace_dir: env::var("WORKSPACE_DIR")
                .unwrap_or_else(|_| "/workspace".to_string()),

            data_dir: env::var("DATA_DIR")
                .unwrap_or_else(|_| "/opt/moatless/data".to_string()),

            upload_dir: env::var("UPLOAD_DIR")
                .unwrap_or_else(|_| "/mnt/uploads".to_string()),

            log_level: env::var("LOG_LEVEL")
                .unwrap_or_else(|_| "info".to_string()),

            #[allow(clippy::expect_used)] // required env var, crash at startup if missing
            backend_grpc_url: env::var("BACKEND_GRPC_URL")
                .expect("BACKEND_GRPC_URL is required"),

            backend_api_url: env::var("BACKEND_API_URL").ok(),

            user_auth_token: env::var("USER_AUTH_TOKEN").ok(),

            agent_config: parsed.agent_config,

            claude_cli_path: env::var("SANDBOX_CLAUDE_EXECUTABLE_PATH")
                .unwrap_or_else(|_| "/usr/local/bin/claude".to_string()),

            home_dir,
            claude_config_dir,
            git_clone_url: env::var("GIT_CLONE_URL").ok(),
            git_branch: env::var("GIT_BRANCH").ok(),
            git_provider: env::var("GIT_PROVIDER").unwrap_or_else(|_| "github".to_string()),
            repo_config_raw: env::var("REPO_CONFIG").ok(),
            setup_commands: parsed.setup_commands,
            plugin_ids: parsed.plugin_ids,
        }
    }
}

/// Parse REPO_CONFIG JSON env var into server configs, agent config, and setup commands.
fn parse_repo_config() -> ParsedRepoConfig {
    let raw = match env::var("REPO_CONFIG") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            return ParsedRepoConfig {
                agent_config: AgentConfig::default(),
                setup_commands: Vec::new(),
                plugin_ids: Vec::new(),
            };
        }
    };

    let repo_config: RepoConfigFile = match serde_json::from_str(&raw) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse REPO_CONFIG");
            return ParsedRepoConfig {
                agent_config: AgentConfig::default(),
                setup_commands: Vec::new(),
                plugin_ids: Vec::new(),
            };
        }
    };

    let agent_config = match repo_config.agent_config {
        Some(ac) => AgentConfig {
            plugins: ac.plugins,
            plugin_paths: ac.plugin_paths,
            timeout: ac.timeout,
            model: ac.model,
            system_prompt: ac.system_prompt,
        },
        None => AgentConfig::default(),
    };

    let setup_commands = repo_config
        .code_execution
        .map(|ce| ce.setup_commands)
        .unwrap_or_default();

    let plugin_ids = repo_config.plugin_ids.unwrap_or_default();

    ParsedRepoConfig {
        agent_config,
        setup_commands,
        plugin_ids,
    }
}
