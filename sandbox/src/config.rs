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
    /// The Task's selected Agent harness, fixed for the Sandbox lifetime and
    /// delivered at construction via the `AGENT_TYPE` env (ADR-0004). `None` on
    /// a legacy pod provisioned before this env existed — the actor then falls
    /// back to the first Message's agent type as migration input. When set it is
    /// validated at load, so an unsupported value fails startup instead of
    /// silently running Claude.
    pub agent_type: Option<String>,
    /// Workspace directory — the persisted *container* dir (e.g.
    /// `/opt/moatless/workspace`). This is the agent's cwd and the root of the
    /// S3 workspace snapshot: **every git repo under this dir** is snapshotted
    /// (the primary repo in [`Self::repo_dir`] *and* any sibling repos the agent
    /// cloned alongside it), keyed by workspace-relative path.
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
    /// Long-lived user auth token, when delivered via the legacy `USER_AUTH_TOKEN`
    /// pod-spec env. On K8s this is now normally absent — the token arrives over
    /// the `GetCredentials` channel instead — but it is still read as a fallback
    /// for backward compatibility and for Docker mode.
    pub user_auth_token: Option<String>,
    /// Short-lived bootstrap token (env `SANDBOX_BOOTSTRAP_TOKEN`) used to
    /// authenticate only the initial `GetCredentials` call, after which the
    /// sandbox swaps to the long-lived user token from that response.
    pub bootstrap_token: Option<String>,
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

/// Read the Task's fixed Agent-harness selection from `AGENT_TYPE`.
///
/// Returns `None` when the env is absent or empty (a legacy pod). Any non-empty
/// value is preserved verbatim — including an unsupported one. Validation is not
/// done here: an unsupported harness must surface as a classified
/// [`HarnessError::InvalidConfiguration`](crate::agent::harness::HarnessError)
/// at the construction seam (`build_harness`, driven from `spawn_agent`), not as
/// a config-load `panic!` that aborts the pod before the Sandbox can publish the
/// failure. Preserving the raw string keeps Claude transport/model variants
/// flowing through the same normalization path and lets the seam reject a truly
/// unsupported selection with a diagnostic.
fn resolve_agent_type_env() -> Option<String> {
    let raw = env::var("AGENT_TYPE").ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw.to_string())
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

            #[allow(clippy::expect_used)] // required env var, crash at startup if missing
            task_id: env::var("TASK_ID").expect("TASK_ID is required"),

            agent_type: resolve_agent_type_env(),

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

            bootstrap_token: env::var("SANDBOX_BOOTSTRAP_TOKEN").ok(),

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
                .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
            permission_mode: env::var("SANDBOX_PERMISSION_MODE")
                .ok()
                .and_then(|v| parse_permission_mode(&v))
                .unwrap_or(PermissionMode::BypassPermissions),
            workspace_sync_enabled: env::var("WORKSPACE_SYNC_ENABLED")
                .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),

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

    /// Filesystem path where the long-lived user auth token is persisted after it
    /// is fetched over `GetCredentials`.
    ///
    /// Written under `workspace_dir` (`/opt/moatless`), which on the default tier
    /// is an EBS-backed PVC. That volume survives not just container restarts but
    /// full **pod replacement** (eviction, node death) — which does not call
    /// `provision()` and so never re-mints tokens. A replacement pod can therefore
    /// recover with this persisted long-lived token even after its spec's
    /// short-lived bootstrap token has expired.
    ///
    /// On the ephemeral (no-PVC) tier `workspace_dir` is a node-local emptyDir
    /// that dies with the pod, so this degrades to container-restart durability
    /// only — no worse than the legacy `data_dir` location.
    pub fn persisted_user_token_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.workspace_dir)
            .join(".moatless")
            .join(".user_auth_token")
    }

    /// Legacy persisted-token location under `data_dir` (the `shared-data`
    /// emptyDir). Read as a fallback so a token written by an older sandbox build
    /// is still honoured after an image upgrade.
    fn legacy_persisted_user_token_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.data_dir).join(".user_auth_token")
    }

    /// Resolve the token the sandbox should present on its very first gRPC calls,
    /// before `GetCredentials` delivers a fresh long-lived token.
    ///
    /// Resolution order (most to least durable):
    /// 1. A previously persisted long-lived token (survives container restart, and
    ///    pod replacement on the PVC tier). The PVC-backed path is preferred, with
    ///    the legacy `data_dir` path read as a fallback.
    /// 2. The legacy `USER_AUTH_TOKEN` env, when present. During the transition
    ///    (`SANDBOX_EMIT_LEGACY_USER_TOKEN=true`) this long-lived token is still
    ///    on the pod spec; preferring it keeps stop/start and pod reschedules
    ///    working even after the short-lived bootstrap token would have expired,
    ///    at no security cost (it is on the spec either way in this phase).
    /// 3. The short-lived bootstrap token — the sole auth secret on the pod spec
    ///    once legacy emission is turned off (`SANDBOX_EMIT_LEGACY_USER_TOKEN=false`).
    pub fn resolve_initial_auth_token(&self) -> Option<String> {
        self.resolve_initial_auth_token_with_source()
            .map(|(token, _)| token)
    }

    /// Like [`Self::resolve_initial_auth_token`] but also reports which source the
    /// token came from, so init can tell an expired-bootstrap failure (permanent —
    /// only re-provisioning helps) apart from a transient backend outage.
    pub fn resolve_initial_auth_token_with_source(
        &self,
    ) -> Option<(String, InitialAuthTokenSource)> {
        for path in [
            self.persisted_user_token_path(),
            self.legacy_persisted_user_token_path(),
        ] {
            if let Ok(token) = std::fs::read_to_string(&path) {
                let token = token.trim();
                if !token.is_empty() {
                    return Some((token.to_string(), InitialAuthTokenSource::Persisted));
                }
            }
        }
        if let Some(token) = self.user_auth_token.clone() {
            return Some((token, InitialAuthTokenSource::LegacyEnv));
        }
        self.bootstrap_token
            .clone()
            .map(|token| (token, InitialAuthTokenSource::Bootstrap))
    }
}

/// Where the sandbox's initial backend-auth token came from — determines whether
/// an `Unauthenticated` credential-fetch failure is recoverable.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum InitialAuthTokenSource {
    /// Long-lived token restored from disk (survives restart; survives pod
    /// replacement on the PVC tier).
    Persisted,
    /// Legacy long-lived `USER_AUTH_TOKEN` env, still on the spec during the
    /// transition (`SANDBOX_EMIT_LEGACY_USER_TOKEN=true`).
    LegacyEnv,
    /// Short-lived bootstrap token — the only auth secret on the spec once legacy
    /// emission is off. If rejected, it has almost certainly expired and a pod
    /// restart would present the same expired env, so re-provisioning is required.
    Bootstrap,
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
            agent_type: None,
            workspace_dir: "/tmp/workspace".to_string(),
            repo_dir: "/tmp/workspace/repo".to_string(),
            data_dir: "/tmp/data".to_string(),
            upload_dir: "/tmp/uploads".to_string(),
            log_level: "info".to_string(),
            backend_grpc_url: "http://backend:50051".to_string(),
            backend_api_url: None,
            user_auth_token: None,
            bootstrap_token: None,
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
    use super::test_support::minimal_config;
    use super::{derive_repo_name, parse_permission_mode, InitialAuthTokenSource, PermissionMode};

    /// Point both persisted-token dirs at empty temp dirs so resolution falls
    /// through to the env-supplied tokens deterministically.
    fn config_without_persisted_token() -> (super::Config, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = minimal_config();
        config.workspace_dir = tmp.path().display().to_string();
        config.data_dir = tmp.path().display().to_string();
        (config, tmp)
    }

    #[test]
    fn resolve_initial_auth_token_prefers_legacy_over_bootstrap() {
        // Phase 1: the long-lived legacy token is still on the spec — prefer it
        // for reliability across reschedules; bootstrap stays as the fallback.
        let (mut config, _tmp) = config_without_persisted_token();
        config.bootstrap_token = Some("boot".to_string());
        config.user_auth_token = Some("legacy".to_string());
        assert_eq!(
            config.resolve_initial_auth_token_with_source(),
            Some(("legacy".to_string(), InitialAuthTokenSource::LegacyEnv))
        );
    }

    #[test]
    fn resolve_initial_auth_token_uses_bootstrap_when_legacy_absent() {
        // Phase 2: legacy emission turned off — bootstrap is the only spec secret.
        let (mut config, _tmp) = config_without_persisted_token();
        config.bootstrap_token = Some("boot".to_string());
        config.user_auth_token = None;
        assert_eq!(
            config.resolve_initial_auth_token_with_source(),
            Some(("boot".to_string(), InitialAuthTokenSource::Bootstrap))
        );
    }

    #[test]
    fn resolve_initial_auth_token_prefers_persisted_file() {
        // The persisted token lives under the PVC-backed workspace dir and wins
        // over both the legacy env and the bootstrap token.
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = minimal_config();
        config.workspace_dir = tmp.path().display().to_string();
        config.data_dir = tmp.path().display().to_string();
        config.bootstrap_token = Some("boot".to_string());
        config.user_auth_token = Some("legacy".to_string());
        let path = config.persisted_user_token_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "persisted-tok\n").unwrap();
        assert_eq!(
            config.resolve_initial_auth_token_with_source(),
            Some((
                "persisted-tok".to_string(),
                InitialAuthTokenSource::Persisted
            ))
        );
    }

    #[test]
    fn resolve_initial_auth_token_reads_legacy_data_dir_path() {
        // A token persisted by an older sandbox build (under data_dir) is still
        // honoured after an image upgrade that moved the primary path to the PVC.
        let ws = tempfile::TempDir::new().unwrap();
        let data = tempfile::TempDir::new().unwrap();
        let mut config = minimal_config();
        config.workspace_dir = ws.path().display().to_string();
        config.data_dir = data.path().display().to_string();
        config.bootstrap_token = Some("boot".to_string());
        std::fs::write(config.legacy_persisted_user_token_path(), "old-persisted\n").unwrap();
        assert_eq!(
            config.resolve_initial_auth_token_with_source(),
            Some((
                "old-persisted".to_string(),
                InitialAuthTokenSource::Persisted
            ))
        );
    }

    #[test]
    fn parse_permission_mode_accepts_known_values() {
        assert_eq!(
            parse_permission_mode("default"),
            Some(PermissionMode::Default)
        );
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
        assert_eq!(
            parse_permission_mode("dontAsk"),
            Some(PermissionMode::DontAsk)
        );
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
        assert_eq!(
            derive_repo_name("https://github.com/owner/repo.git"),
            "repo"
        );
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
