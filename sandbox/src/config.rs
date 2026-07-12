//! Configuration loaded from environment variables.
//!
//! The sandbox runs inside a sandbox pod and receives its configuration
//! via environment variables set by the orchestrator.

use std::env;

use claude_agent_sdk::PermissionMode;
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

/// Proxy configuration for constructing preview URLs.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Proxy domain (e.g., "preview.moatless.xyz")
    pub domain: Option<String>,
    /// Protocol (default: "https")
    pub protocol: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            domain: None,
            protocol: "https".to_string(),
        }
    }
}

/// Server info parsed from REPO_CONFIG, used for system prompt.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub start_commands: Option<Vec<String>>,
    #[serde(default)]
    pub auto_reload: bool,
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
    #[serde(default)]
    plugin_repos: Option<Vec<String>>,
    #[serde(default)]
    servers: Option<Vec<ServerInfo>>,
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
    plugin_repos: Vec<String>,
    servers: Vec<ServerInfo>,
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
    /// Workspace directory — the persisted *container* dir (e.g.
    /// `/opt/moatless/workspace`). This is the agent's cwd and the root of the
    /// S3 workspace snapshot; the primary repo lives in [`Self::repo_dir`], a
    /// subdirectory of this.
    pub workspace_dir: String,
    /// Repository directory — where the primary repo is cloned, a subdirectory
    /// of [`Self::workspace_dir`] named after the repo (e.g.
    /// `/opt/moatless/workspace/<repo-name>`). All repo-scoped operations (git
    /// status/diff, the file API, setup commands, dev servers) target this.
    /// Falls back to `workspace_dir` when there is no clone URL.
    pub repo_dir: String,
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
    /// Path to the Codex CLI binary
    pub codex_cli_path: String,
    /// Proxy configuration for preview URLs
    pub proxy: ProxyConfig,
    /// Server configurations parsed from REPO_CONFIG
    pub servers: Vec<ServerInfo>,
    /// Frontend URL for constructing task URLs (e.g., "https://moatless.io")
    pub frontend_url: Option<String>,
    /// Global default Claude model (env: DEFAULT_CLAUDE_CODE_MODEL), used when neither request nor repo config set one
    pub default_model: Option<String>,
    /// Global default Codex model (env: DEFAULT_CODEX_MODEL), used when neither request nor repo config set one.
    /// Falls through to [`crate::agent::codex_exec::DEFAULT_CODEX_MODEL`] at call time if unset.
    pub default_codex_model: Option<String>,
    /// Use the tmux-based Claude Code SDK transport instead of subprocess/NDJSON (env: SANDBOX_USE_TMUX).
    pub use_tmux: bool,
    /// Permission mode for the Claude CLI in normal (non-plan) mode (env:
    /// `SANDBOX_PERMISSION_MODE`). Defaults to `bypassPermissions` so network
    /// tools like WebSearch/WebFetch aren't denied (they are not auto-approved
    /// by `acceptEdits` and the CLI never routes them through the stdio
    /// permission handler). Plan-mode requests always override this with
    /// `plan`. Ignored in tmux mode, which always uses
    /// `--dangerously-skip-permissions`. Accepts the CLI spellings: `default`,
    /// `acceptEdits`, `bypassPermissions`, `plan`, `delegate`, `dontAsk`.
    pub permission_mode: PermissionMode,
    /// Whether the `s3` storage backend's workspace sync is enabled (env:
    /// `WORKSPACE_SYNC_ENABLED`). Set by the backend only for `storage_mode:
    /// "s3"` pods; when true, the sandbox restores the git working tree from S3
    /// on boot and periodically syncs it back via backend-minted presigned URLs.
    pub workspace_sync_enabled: bool,

    // ── Init-related fields ────────────────────────────────────────
    /// Home directory (default: "/home/moatless")
    pub home_dir: String,
    /// Codex home directory (default: "$HOME/.codex")
    pub codex_home_dir: String,
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
    /// Plugin git repo URLs from REPO_CONFIG.pluginRepos
    pub plugin_repos: Vec<String>,
}

/// Derive the repo subdirectory name from a clone URL: the last path segment
/// with any trailing `.git` and slashes stripped. Falls back to `"repo"` for
/// URLs that don't yield a usable segment.
///
/// Handles `https://host/owner/repo(.git)` and `git@host:owner/repo(.git)`.
pub(crate) fn derive_repo_name(url: &str) -> String {
    let name = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".git");
    if name.is_empty() {
        "repo".to_string()
    } else {
        name.to_string()
    }
}

/// Parse the `SANDBOX_PERMISSION_MODE` env value into a [`PermissionMode`].
///
/// Accepts the CLI's camelCase spellings case-insensitively (`default`,
/// `acceptEdits`, `bypassPermissions`, `plan`, `delegate`, `dontAsk`) plus a
/// couple of snake_case aliases. Returns `None` for empty or unrecognised
/// values so the caller falls back to the default.
fn parse_permission_mode(raw: &str) -> Option<PermissionMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "default" => Some(PermissionMode::Default),
        "acceptedits" | "accept_edits" => Some(PermissionMode::AcceptEdits),
        "bypasspermissions" | "bypass" => Some(PermissionMode::BypassPermissions),
        "plan" => Some(PermissionMode::Plan),
        "delegate" => Some(PermissionMode::Delegate),
        "dontask" | "dont_ask" => Some(PermissionMode::DontAsk),
        other => {
            tracing::warn!(
                value = %other,
                "Unrecognised SANDBOX_PERMISSION_MODE; falling back to default"
            );
            None
        }
    }
}

impl Config {
    /// Load configuration from environment variables.
    pub fn from_env() -> Self {
        let parsed = parse_repo_config();

        let home_dir = env::var("HOME").unwrap_or_else(|_| "/home/moatless".to_string());

        // Resolve via the shared helper so the sandbox mirrors the Claude CLI
        // and SDK: `CLAUDE_CONFIG_DIR`, then its `CLAUDE_HOME` alias, then
        // `{home_dir}/.claude` (home_dir keeps the `/home/moatless` fallback).
        let claude_config_dir = claude_agent_sdk::resolve_claude_config_dir(&home_dir)
            .display()
            .to_string();
        let codex_home_dir =
            env::var("CODEX_HOME").unwrap_or_else(|_| format!("{home_dir}/.codex"));

        // The workspace is the persisted container dir; the primary repo is
        // cloned into a named subdirectory of it. Derive `repo_dir` from the
        // clone URL's last path segment, falling back to `workspace_dir` itself
        // when there is no repo to clone.
        let workspace_dir = env::var("WORKSPACE_DIR").unwrap_or_else(|_| "/workspace".to_string());
        let git_clone_url = env::var("GIT_CLONE_URL").ok().filter(|u| !u.is_empty());
        let repo_dir = match git_clone_url {
            Some(ref url) => format!(
                "{}/{}",
                workspace_dir.trim_end_matches('/'),
                derive_repo_name(url)
            ),
            None => workspace_dir.clone(),
        };

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

            workspace_dir,
            repo_dir,

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
            codex_cli_path: env::var("SANDBOX_CODEX_EXECUTABLE_PATH")
                .unwrap_or_else(|_| "/usr/local/bin/codex".to_string()),

            proxy: ProxyConfig {
                domain: env::var("PROXY_DOMAIN").ok(),
                protocol: env::var("PROXY_PROTOCOL")
                    .unwrap_or_else(|_| "https".to_string()),
            },
            servers: parsed.servers,
            frontend_url: env::var("FRONTEND_URL").ok(),
            default_model: env::var("DEFAULT_CLAUDE_CODE_MODEL").ok(),
            default_codex_model: env::var("DEFAULT_CODEX_MODEL").ok(),
            use_tmux: env::var("SANDBOX_USE_TMUX")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            permission_mode: env::var("SANDBOX_PERMISSION_MODE")
                .ok()
                .and_then(|v| parse_permission_mode(&v))
                .unwrap_or(PermissionMode::BypassPermissions),
            workspace_sync_enabled: env::var("WORKSPACE_SYNC_ENABLED")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),

            home_dir,
            codex_home_dir,
            claude_config_dir,
            git_clone_url,
            git_branch: env::var("GIT_BRANCH").ok(),
            git_provider: env::var("GIT_PROVIDER").unwrap_or_else(|_| "github".to_string()),
            repo_config_raw: env::var("REPO_CONFIG").ok(),
            setup_commands: parsed.setup_commands,
            plugin_ids: parsed.plugin_ids,
            plugin_repos: parsed.plugin_repos,
        }
    }
}

/// Test-only helpers for constructing a [`Config`] without env vars.
#[cfg(test)]
pub(crate) mod test_support {
    use super::{AgentConfig, Config, PermissionMode, ProxyConfig};

    /// A minimal [`Config`] with empty/default values, suitable for unit tests
    /// that only exercise a few fields. Callers override what they need.
    pub(crate) fn minimal_config() -> Config {
        Config {
            port: 8001,
            grpc_port: 50052,
            task_id: "test-task".to_string(),
            workspace_dir: "/tmp/workspace".to_string(),
            repo_dir: "/tmp/workspace/repo".to_string(),
            data_dir: "/tmp/data".to_string(),
            upload_dir: "/tmp/uploads".to_string(),
            log_level: "info".to_string(),
            backend_grpc_url: "http://backend:50051".to_string(),
            backend_api_url: None,
            user_auth_token: None,
            agent_config: AgentConfig::default(),
            claude_cli_path: "/usr/local/bin/claude".to_string(),
            codex_cli_path: "/usr/local/bin/codex".to_string(),
            proxy: ProxyConfig::default(),
            servers: Vec::new(),
            frontend_url: None,
            default_model: None,
            default_codex_model: None,
            use_tmux: false,
            permission_mode: PermissionMode::BypassPermissions,
            workspace_sync_enabled: false,
            home_dir: "/tmp/home".to_string(),
            codex_home_dir: "/tmp/home/.codex".to_string(),
            claude_config_dir: "/tmp/home/.claude".to_string(),
            git_clone_url: None,
            git_branch: None,
            git_provider: "github".to_string(),
            repo_config_raw: None,
            setup_commands: Vec::new(),
            plugin_ids: Vec::new(),
            plugin_repos: Vec::new(),
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
                plugin_repos: Vec::new(),
                servers: Vec::new(),
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
                plugin_repos: Vec::new(),
                servers: Vec::new(),
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
    let plugin_repos = repo_config.plugin_repos.unwrap_or_default();
    let servers = repo_config.servers.unwrap_or_default();

    ParsedRepoConfig {
        agent_config,
        setup_commands,
        plugin_ids,
        plugin_repos,
        servers,
    }
}

#[cfg(test)]
mod tests {
    use super::{derive_repo_name, parse_permission_mode, PermissionMode};

    #[test]
    fn parse_permission_mode_accepts_known_values() {
        assert_eq!(parse_permission_mode("default"), Some(PermissionMode::Default));
        assert_eq!(
            parse_permission_mode("acceptEdits"),
            Some(PermissionMode::AcceptEdits)
        );
        assert_eq!(
            parse_permission_mode("bypassPermissions"),
            Some(PermissionMode::BypassPermissions)
        );
        assert_eq!(parse_permission_mode("plan"), Some(PermissionMode::Plan));
        assert_eq!(
            parse_permission_mode("delegate"),
            Some(PermissionMode::Delegate)
        );
        assert_eq!(parse_permission_mode("dontAsk"), Some(PermissionMode::DontAsk));
    }

    #[test]
    fn parse_permission_mode_is_case_insensitive_and_trims() {
        assert_eq!(
            parse_permission_mode("  BYPASSPERMISSIONS  "),
            Some(PermissionMode::BypassPermissions)
        );
        assert_eq!(
            parse_permission_mode("bypass"),
            Some(PermissionMode::BypassPermissions)
        );
    }

    #[test]
    fn parse_permission_mode_rejects_empty_and_unknown() {
        assert_eq!(parse_permission_mode(""), None);
        assert_eq!(parse_permission_mode("   "), None);
        assert_eq!(parse_permission_mode("nonsense"), None);
    }

    #[test]
    fn derive_repo_name_handles_common_url_shapes() {
        assert_eq!(derive_repo_name("https://github.com/owner/repo.git"), "repo");
        assert_eq!(derive_repo_name("https://github.com/owner/repo"), "repo");
        assert_eq!(derive_repo_name("git@github.com:owner/repo.git"), "repo");
        assert_eq!(derive_repo_name("https://host/owner/repo/"), "repo");
        assert_eq!(
            derive_repo_name("https://gitlab.com/group/sub/my-service.git"),
            "my-service"
        );
    }

    #[test]
    fn derive_repo_name_falls_back_for_unusable_urls() {
        assert_eq!(derive_repo_name(""), "repo");
        assert_eq!(derive_repo_name("/"), "repo");
        assert_eq!(derive_repo_name(".git"), "repo");
    }
}
