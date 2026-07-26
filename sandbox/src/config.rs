//! Configuration loaded from environment variables.
//!
//! The sandbox runs inside a sandbox pod and receives its configuration
//! via environment variables set by the orchestrator.

use std::env;

use claude_agent_sdk::PermissionMode;
use moatless_api_schemas::plugin::PluginDelivery;
use moatless_api_schemas::sandbox::{derive_mount_name, RepoCheckout, WORKSPACE_REPOS_ENV};
use serde::Deserialize;

/// Agent-specific configuration parsed from REPO_CONFIG.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct AgentConfig {
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
    plugins: Option<Vec<PluginDelivery>>,
    #[serde(default)]
    servers: Option<Vec<ServerInfo>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoAgentConfig {
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
    plugins: Vec<PluginDelivery>,
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
    /// Repository directory — where the primary repo is cloned: the
    /// [`Self::workspace_dir`] subdirectory named by its placement's mount name,
    /// which defaults to the repo name (e.g.
    /// `/opt/moatless/workspace/<repo-name>`). All repo-scoped operations (git
    /// status/diff, the file API, setup commands, dev servers) target this;
    /// secondary repos in [`Self::repos`] sit beside it. Falls back to
    /// `workspace_dir` when there is no repo to clone.
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
    /// Primary repo's clone URL (env `GIT_CLONE_URL`). Mirrors `repos[0]`.
    pub git_clone_url: Option<String>,
    /// Primary repo's branch (env `GIT_BRANCH`)
    pub git_branch: Option<String>,
    /// Primary repo's git provider (github, gitlab, gitness)
    pub git_provider: String,
    /// Every repository the task's workspace places in this sandbox, primary
    /// first (env `WORKSPACE_REPOS`). Each is cloned into
    /// `{workspace_dir}/{mount_name}`; the primary's directory is
    /// [`Self::repo_dir`].
    ///
    /// A pod provisioned before multi-repo support carries no `WORKSPACE_REPOS`,
    /// so this holds the single checkout reconstructed from the `GIT_*` trio —
    /// or nothing at all, for a sandbox with no repo to clone.
    pub repos: Vec<RepoCheckout>,
    /// Raw REPO_CONFIG JSON (for entrypoint compat)
    pub repo_config_raw: Option<String>,
    /// Setup commands from REPO_CONFIG.codeExecution.setupCommands
    pub setup_commands: Vec<String>,
    /// The plugins to clone, each with the skills to keep, from
    /// REPO_CONFIG.plugins
    pub plugins: Vec<PluginDelivery>,
}

/// Read the workspace's repository checkouts from `WORKSPACE_REPOS`.
///
/// Falls back to the single checkout described by the legacy
/// `GIT_CLONE_URL`/`GIT_BRANCH`/`GIT_PROVIDER` trio when the env is absent (a pod
/// provisioned before multi-repo support) or unparseable — cloning the primary
/// repo is strictly better than cloning nothing. Returns an empty list when
/// there is no repo to clone at all.
///
/// A mount name that is not a single safe path segment is rejected rather than
/// repaired: it is joined onto `workspace_dir`, and the backend already
/// sanitizes, so anything else means a payload we should not act on.
fn load_repo_checkouts(
    git_clone_url: Option<&str>,
    git_branch: Option<&str>,
    git_provider: &str,
) -> Vec<RepoCheckout> {
    parse_repo_checkouts(
        env::var(WORKSPACE_REPOS_ENV).ok().as_deref(),
        git_clone_url,
        git_branch,
        git_provider,
    )
}

/// The env-free half of [`load_repo_checkouts`].
fn parse_repo_checkouts(
    raw: Option<&str>,
    git_clone_url: Option<&str>,
    git_branch: Option<&str>,
    git_provider: &str,
) -> Vec<RepoCheckout> {
    let legacy = || match git_clone_url {
        Some(url) => vec![RepoCheckout {
            clone_url: url.to_string(),
            branch: git_branch.map(str::to_string),
            provider: git_provider.to_string(),
            mount_name: derive_mount_name(url),
            is_primary: true,
        }],
        None => Vec::new(),
    };

    let raw = match raw {
        Some(raw) if !raw.trim().is_empty() => raw,
        _ => return legacy(),
    };
    let parsed: Vec<RepoCheckout> = match serde_json::from_str(raw) {
        Ok(parsed) => parsed,
        Err(e) => {
            tracing::error!(
                error = %e,
                "Failed to parse WORKSPACE_REPOS; falling back to the primary repo only"
            );
            return legacy();
        }
    };

    let (usable, rejected): (Vec<_>, Vec<_>) = parsed
        .into_iter()
        .partition(|repo| is_safe_mount_name(&repo.mount_name) && !repo.clone_url.is_empty());
    for repo in &rejected {
        tracing::error!(
            mount_name = %repo.mount_name,
            "Skipping workspace repo with an unusable mount name or clone URL"
        );
    }
    if usable.is_empty() {
        return legacy();
    }
    usable
}

/// The primary checkout in a list, or `None` when there is nothing to clone.
///
/// Falls back to the first entry if nothing is flagged, so a malformed list
/// still yields a `repo_dir` rather than collapsing onto the workspace root.
pub(crate) fn primary_checkout(repos: &[RepoCheckout]) -> Option<&RepoCheckout> {
    repos.iter().find(|repo| repo.is_primary).or(repos.first())
}

/// Whether a mount name is a single path segment safe to join onto the workspace
/// directory.
fn is_safe_mount_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
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

        // The workspace is the persisted container dir; every repository the
        // task's workspace declares is cloned into a named subdirectory of it.
        // `repo_dir` is the primary checkout's subdirectory — the target of all
        // repo-scoped operations — falling back to `workspace_dir` itself when
        // there is no repo to clone.
        let workspace_dir = env::var("WORKSPACE_DIR").unwrap_or_else(|_| "/workspace".to_string());
        let git_clone_url = env::var("GIT_CLONE_URL").ok().filter(|u| !u.is_empty());
        let git_branch = env::var("GIT_BRANCH").ok();
        let git_provider = env::var("GIT_PROVIDER").unwrap_or_else(|_| "github".to_string());
        let repos = load_repo_checkouts(
            git_clone_url.as_deref(),
            git_branch.as_deref(),
            &git_provider,
        );
        let repo_dir = primary_checkout(&repos)
            .map_or_else(|| workspace_dir.clone(), |repo| repo.dir(&workspace_dir));

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
            git_branch,
            git_provider,
            repos,
            repo_config_raw: env::var("REPO_CONFIG").ok(),
            setup_commands: parsed.setup_commands,
            plugins: parsed.plugins,
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
            repos: Vec::new(),
            repo_config_raw: None,
            setup_commands: Vec::new(),
            plugins: Vec::new(),
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
                plugins: Vec::new(),
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
                plugins: Vec::new(),
                servers: Vec::new(),
            };
        }
    };

    let agent_config = match repo_config.agent_config {
        Some(ac) => AgentConfig {
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

    let plugins = repo_config.plugins.unwrap_or_default();
    let servers = repo_config.servers.unwrap_or_default();

    ParsedRepoConfig {
        agent_config,
        setup_commands,
        plugins,
        servers,
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::minimal_config;
    use super::{
        is_safe_mount_name, parse_permission_mode, parse_repo_checkouts, primary_checkout,
        InitialAuthTokenSource, PermissionMode,
    };

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

    /// A pod provisioned before multi-repo support carries no `WORKSPACE_REPOS`;
    /// it must still clone its primary repo into the same directory as before.
    #[test]
    fn absent_workspace_repos_falls_back_to_the_legacy_single_checkout() {
        let repos = parse_repo_checkouts(
            None,
            Some("https://github.com/owner/repo.git"),
            Some("main"),
            "github",
        );
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].mount_name, "repo");
        assert_eq!(repos[0].branch.as_deref(), Some("main"));
        assert!(repos[0].is_primary);
        assert_eq!(
            primary_checkout(&repos).unwrap().dir("/opt/moatless/workspace"),
            "/opt/moatless/workspace/repo"
        );
    }

    #[test]
    fn no_clone_url_at_all_yields_no_checkouts() {
        assert!(parse_repo_checkouts(None, None, None, "github").is_empty());
    }

    #[test]
    fn workspace_repos_env_supplies_every_checkout() {
        let raw = r#"[
            {"cloneUrl":"https://github.com/acme/api.git","branch":"main","provider":"github","mountName":"api","isPrimary":true},
            {"cloneUrl":"https://gitlab.com/acme/shared.git","provider":"gitlab","mountName":"lib"}
        ]"#;
        let repos = parse_repo_checkouts(
            Some(raw),
            Some("https://github.com/acme/api.git"),
            Some("main"),
            "github",
        );
        assert_eq!(repos.len(), 2);
        assert_eq!(
            primary_checkout(&repos).unwrap().dir("/opt/moatless/workspace"),
            "/opt/moatless/workspace/api"
        );
        assert_eq!(repos[1].provider, "gitlab");
        assert_eq!(repos[1].branch, None);
        assert!(!repos[1].is_primary);
    }

    /// Mount names are joined onto the workspace dir, so an entry that escapes it
    /// is dropped rather than repaired — the backend already sanitizes.
    #[test]
    fn checkouts_with_unusable_mount_names_are_dropped() {
        let raw = r#"[
            {"cloneUrl":"https://github.com/acme/api.git","provider":"github","mountName":"api","isPrimary":true},
            {"cloneUrl":"https://github.com/acme/evil.git","provider":"github","mountName":"../escape"},
            {"cloneUrl":"","provider":"github","mountName":"empty"}
        ]"#;
        let repos = parse_repo_checkouts(Some(raw), None, None, "github");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].mount_name, "api");
    }

    /// Unparseable JSON must not strand the sandbox with nothing checked out.
    #[test]
    fn malformed_workspace_repos_falls_back_to_the_primary() {
        let repos = parse_repo_checkouts(
            Some("not json"),
            Some("https://github.com/owner/repo.git"),
            None,
            "github",
        );
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].mount_name, "repo");
    }

    #[test]
    fn rejects_mount_names_that_are_not_a_single_segment() {
        assert!(is_safe_mount_name("api"));
        assert!(is_safe_mount_name("api-2"));
        assert!(!is_safe_mount_name(""));
        assert!(!is_safe_mount_name("."));
        assert!(!is_safe_mount_name(".."));
        assert!(!is_safe_mount_name("a/b"));
        assert!(!is_safe_mount_name("a\\b"));
    }

    /// Nothing flagged primary still yields a `repo_dir` rather than collapsing
    /// onto the workspace root.
    #[test]
    fn primary_checkout_falls_back_to_the_first_entry() {
        let raw = r#"[{"cloneUrl":"https://github.com/acme/api.git","provider":"github","mountName":"api"}]"#;
        let repos = parse_repo_checkouts(Some(raw), None, None, "github");
        assert_eq!(primary_checkout(&repos).unwrap().mount_name, "api");
        assert!(primary_checkout(&[]).is_none());
    }
}
