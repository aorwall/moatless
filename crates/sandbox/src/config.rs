//! Configuration loaded from environment variables.
//!
//! The sandbox runs inside a sandbox pod and receives its configuration via
//! environment variables set by the orchestrator.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use claude_agent_sdk::PermissionMode;
use moatless_workspace_types::plugin::PluginDelivery;
use moatless_workspace_types::workspace::{
    LanguageServer, PROXIED_PROVIDER, RepoCheckout, WORKSPACE_LIMIT_ENV, WORKSPACE_REPOS_ENV,
    proxy_clone_repository_id, proxy_clone_url,
};
use serde::Deserialize;

pub use crate::http_config::SandboxHttpConfig;

/// Sandbox-private runtime directory.
///
/// Outside `workspace_dir`: holds the credential broker socket and the
/// persisted user auth token, neither of which may reach the workspace snapshot
/// or server sidecars.
pub const RUNTIME_DIR: &str = "/run/moatless";

const GITHUB_PROVIDER: &str = moatless_types::git_host::RepositoryProvider::Github.as_str();

/// Deadline a `moat cmd` command takes when it declares none.
pub const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 4 * 60 * 60;

/// Agent-specific configuration parsed from `REPO_CONFIG`.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct AgentConfig {
    pub timeout: Option<u64>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    /// Whether the repo's root instruction file is inlined into the system
    /// prompt. `None` means unset, which enables it — see
    /// [`AgentConfig::frontload_instructions_enabled`].
    pub frontload_instructions: Option<bool>,
}

impl AgentConfig {
    /// Whether to inline repo root instruction files into the system prompt;
    /// defaults to on when unset.
    ///
    /// Resolved in order: `SANDBOX_FRONTLOAD_REPO_INSTRUCTIONS`, then
    /// `REPO_CONFIG`'s `frontloadInstructions`, then this default.
    pub fn frontload_instructions_enabled(&self) -> bool {
        self.frontload_instructions.unwrap_or(true)
    }
}

/// Code execution configuration from `REPO_CONFIG`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CodeExecutionConfig {
    #[serde(default)]
    pub setup_commands: Vec<String>,
}

/// Partial `REPO_CONFIG` JSON structure — we only parse the fields we need.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoConfigFile {
    #[serde(default)]
    agent_config: Option<RepoAgentConfig>,
    /// `frontloadInstructions` at the top level, the shape provisioning emits
    /// (flat keys, like `systemPrompt`, not nested under `agentConfig`). Takes
    /// precedence over the nested spelling.
    #[serde(default)]
    frontload_instructions: Option<bool>,
    #[serde(default)]
    code_execution: Option<CodeExecutionConfig>,
    #[serde(default)]
    plugins: Option<Vec<PluginDelivery>>,
    /// Ids rather than [`LanguageServer`], so an id this sandbox build does not
    /// know drops alone instead of failing the whole parse.
    #[serde(default)]
    language_servers: Option<Vec<String>>,
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
    /// `frontloadInstructions` — omit to inline the repo's root instruction
    /// file, `false` to fall back to listing its path.
    #[serde(default)]
    frontload_instructions: Option<bool>,
}

/// Parsed `REPO_CONFIG` result.
struct ParsedRepoConfig {
    agent_config: AgentConfig,
    setup_commands: Vec<String>,
    plugins: Vec<PluginDelivery>,
    language_servers: Vec<LanguageServer>,
}

/// Main sandbox configuration.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    /// HTTP port (default: 8001) — kept for K8s health probes
    pub port: u16,
    /// gRPC port for sandbox service (default: 50052)
    pub grpc_port: u16,
    /// Sandbox ID (required for new pods; falls back to `TASK_ID` for older
    /// local fixtures).
    pub sandbox_id: String,
    /// Initial member Task ID.
    pub task_id: String,
    /// The Task's selected Agent harness, fixed for the Member's lifetime.
    /// `None` means no valid selection, surfaced as invalid config by the agent
    /// lifecycle owner rather than defaulting silently to Claude.
    pub agent_profile: Option<String>,
    /// Workspace directory — the persisted *container* dir (e.g.
    /// `/opt/moatless/workspace`), the agent's cwd. **Every git repo under this
    /// dir** is snapshotted to S3, keyed by workspace-relative path.
    pub workspace_dir: String,
    /// Repository directory — the [`Self::workspace_dir`] subdirectory where
    /// the primary repo is cloned. All repo-scoped operations target this;
    /// falls back to `workspace_dir` when there is no repo to clone.
    pub repo_dir: String,
    /// Data directory for server artifacts
    pub data_dir: String,
    /// Sandbox-private runtime directory ([`RUNTIME_DIR`]), for credential
    /// material that must not reach repo code or the workspace snapshot.
    pub runtime_dir: String,
    /// Where a message's attached uploads are written (env `UPLOAD_DIR`,
    /// matching the backend prompt's "Saved to:" line). Defaults under
    /// [`Self::workspace_dir`], surviving a restart via the snapshot.
    pub upload_dir: String,
    /// Log level
    pub log_level: String,
    /// Backend gRPC URL (e.g. "<http://backend:50051>") — required
    pub backend_grpc_url: String,
    /// Backend API URL for HTTP fallback calls.
    pub backend_api_url: Option<String>,
    /// Long-lived user auth token from `USER_AUTH_TOKEN`, when present. K8s
    /// pods normally receive the long-lived token over `GetCredentials`; Docker
    /// and compatibility paths may still provide this env.
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
    /// Frontend URL for constructing task URLs (e.g., "<https://moatless.io>")
    pub frontend_url: Option<String>,
    /// Global default Claude model (env: `DEFAULT_CLAUDE_CODE_MODEL`), used
    /// when neither request nor repo config set one
    pub default_model: Option<String>,
    /// Base URL of an OpenAI-compatible proxy to route Codex through (env:
    /// `OPENAI_BASE_URL`), including the `/v1` suffix.
    ///
    /// Also suppresses the `auth.json` write, which would overwrite the user's
    /// Codex OAuth credential.
    pub openai_base_url: Option<String>,
    /// Use the tmux-based Claude Code SDK transport instead of
    /// subprocess/NDJSON (env: `SANDBOX_USE_TMUX`).
    pub use_tmux: bool,
    /// Deadline applied to a `moat cmd` command that declares none (env:
    /// `SANDBOX_COMMAND_DEFAULT_TIMEOUT_SECS`, default 4h); a command's own
    /// value overrides it either direction, past this default too.
    pub command_default_timeout_secs: u64,
    /// Permission mode for the Claude CLI outside plan mode (env:
    /// `SANDBOX_PERMISSION_MODE`); defaults to `bypassPermissions` so network
    /// tools aren't denied, `plan` in plan mode; unused in tmux mode.
    pub permission_mode: PermissionMode,
    /// Whether this sandbox snapshots its workspace to S3 (env:
    /// `WORKSPACE_SYNC_ENABLED`); when true, restores from S3 on boot and syncs
    /// back via backend-minted presigned URLs.
    pub workspace_sync_enabled: bool,
    /// Whether the volume under `WORKSPACE_DIR` is durable (env:
    /// `WORKSPACE_RESTORE_IF_EMPTY`). When true, restore only applies to an
    /// empty volume, and the periodic upload loop is disabled.
    pub workspace_restore_if_empty: bool,
    /// Whether the task runs headless (env: `SANDBOX_HEADLESS`), i.e.
    /// unwatched. Removes tools that would park the turn on an answer nobody
    /// can give (`AskUserQuestion`, `EnterPlanMode`, `ExitPlanMode`).
    pub headless: bool,
    /// Credential refresh period (env: `CREDENTIAL_REFRESH_INTERVAL_SECS`,
    /// default 900s; `0` or garbage keeps the default).
    pub credential_refresh_interval: Duration,
    /// Transcript upload period (env: `TRANSCRIPT_SYNC_INTERVAL_SECS`, default
    /// 60s); `None` when set to `0`, which disables only the periodic loop.
    pub transcript_sync_interval: Option<Duration>,
    /// Workspace upload period (env: `WORKSPACE_SYNC_INTERVAL_SECS`, default
    /// 60s); `None` when set to `0`, which disables only the periodic loop.
    pub workspace_sync_interval: Option<Duration>,
    /// The workspace volume's size limit as the pod builder wrote it (`20Gi`),
    /// or `None` in a runtime that caps nothing.
    pub workspace_limit: Option<String>,
    /// The shell a terminal runs (env: `SHELL`, default `/bin/bash`).
    pub shell: String,

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
    /// Primary repo's git provider (github, gitlab, forgejo)
    pub git_provider: String,
    /// Resolved checkouts supplied in `WORKSPACE_REPOS`, primary first.
    pub repos: Vec<RepoCheckout>,
    /// The GitHub URL each checkout cloning through the backend proxy stands
    /// in for, by mount name. The clone records it as the
    /// [`GITHUB_REMOTE`](crate::init::git::GITHUB_REMOTE) remote.
    pub github_upstreams: HashMap<String, String>,
    /// Raw `REPO_CONFIG` JSON (for entrypoint compat)
    pub repo_config_raw: Option<String>,
    /// Setup commands from `REPO_CONFIG.codeExecution.setupCommands`
    pub setup_commands: Vec<String>,
    /// The plugins to clone, each with the skills to keep, from
    /// `REPO_CONFIG.plugins`
    pub plugins: Vec<PluginDelivery>,
    /// The language servers to give the agent, from
    /// `REPO_CONFIG.languageServers`
    pub language_servers: Vec<LanguageServer>,
    /// `PATH`, where init looks for each language server's binary.
    pub search_path: Option<std::ffi::OsString>,
    /// The timeouts each HTTP client this sandbox builds is given.
    pub http: SandboxHttpConfig,
}

/// Read the resolved checkouts supplied by the backend, routed as
/// [`route_through_proxy`] does, with the GitHub URLs it replaced.
fn load_repo_checkouts(
    git_clone_url: Option<&str>,
    backend_api_url: Option<&str>,
) -> Result<(Vec<RepoCheckout>, HashMap<String, String>), String> {
    let mut repos =
        parse_repo_checkouts(env::var(WORKSPACE_REPOS_ENV).ok().as_deref(), git_clone_url)?;
    let github_upstreams = route_through_proxy(&mut repos, backend_api_url);
    Ok((repos, github_upstreams))
}

/// Decode resolved checkouts without consulting process environment.
fn parse_repo_checkouts(
    raw: Option<&str>,
    git_clone_url: Option<&str>,
) -> Result<Vec<RepoCheckout>, String> {
    let Some(raw) = raw.filter(|raw| !raw.trim().is_empty()) else {
        return if git_clone_url.is_some() {
            Err("WORKSPACE_REPOS metadata is missing; update the backend and reprovision the sandbox.".to_string())
        } else {
            Ok(Vec::new())
        };
    };
    let mut repos: Vec<RepoCheckout> = serde_json::from_str(raw)
        .map_err(|error| format!("Invalid WORKSPACE_REPOS metadata; update the backend and reprovision the sandbox: {error}"))?;
    repos.retain(|repo| {
        let usable = is_safe_mount_name(&repo.mount_name) && !repo.clone_url.is_empty();
        if !usable { tracing::error!(mount_name = %repo.mount_name, "Skipping checkout with an unusable mount name or clone URL"); }
        usable
    });
    Ok(repos)
}

/// Point every checkout [`proxied_clone_url`] names at the backend proxy.
///
/// Returns the `clone_url` each rewritten GitHub checkout had, by mount name:
/// `gh` and the push watcher still need the GitHub repository the proxy
/// stands in for.
fn route_through_proxy(
    repos: &mut [RepoCheckout],
    backend_api_url: Option<&str>,
) -> HashMap<String, String> {
    let mut github_upstreams = HashMap::new();
    for repo in repos {
        if let Some(url) = proxied_clone_url(repo, backend_api_url) {
            let upstream = std::mem::replace(&mut repo.clone_url, url);
            if repo.provider == GITHUB_PROVIDER {
                github_upstreams.insert(repo.mount_name.clone(), upstream);
            }
            repo.uses_ssh = false;
        }
    }
    github_upstreams
}

/// Split a URL into its scheme and its `host[:port]`, dropping any path.
fn scheme_and_authority(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split('/').next().filter(|a| !a.is_empty())?;
    Some((scheme, authority))
}

/// The backend-proxied remote to clone this checkout from: every Forgejo and
/// every HTTPS GitHub checkout, so git asks the broker for the task token
/// rather than a provider token. SSH GitHub keeps its own host and keys.
/// `None` with no `BACKEND_API_URL` or repository id.
fn proxied_clone_url(repo: &RepoCheckout, backend_api_url: Option<&str>) -> Option<String> {
    let proxied =
        repo.provider == PROXIED_PROVIDER || (repo.provider == GITHUB_PROVIDER && !repo.uses_ssh);
    if !proxied || repo.repository_id.is_empty() {
        return None;
    }
    let base = backend_api_url.filter(|url| !url.is_empty())?;
    Some(proxy_clone_url(base, &repo.repository_id))
}

/// The primary checkout in a list, or `None` when there is nothing to clone.
///
/// Falls back to the first entry if nothing is flagged, so a malformed list
/// still yields a `repo_dir`.
pub(crate) fn primary_checkout(repos: &[RepoCheckout]) -> Option<&RepoCheckout> {
    repos.iter().find(|repo| repo.is_primary).or(repos.first())
}

/// Whether a mount name is a single path segment safe to join onto the
/// workspace directory.
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
/// Accepts the CLI's camelCase spellings case-insensitively, plus a few
/// `snake_case` aliases; `None` for empty or unrecognised values.
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

/// `None` when absent or empty; an unsupported value is preserved verbatim,
/// surfacing as
/// [`HarnessError::InvalidConfiguration`](crate::agent::harness::HarnessError)
/// rather than a config-load panic.
fn resolve_agent_profile_env() -> Option<String> {
    let raw = env::var("AGENT_PROFILE").ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw.to_string())
}

/// The default credential refresh period.
const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 900;

/// The default transcript and workspace upload period.
///
/// For the workspace it bounds the loss from an involuntary mid-turn kill, the
/// one stop neither turn completion nor the shutdown flush covers. Affordable
/// because a snapshot ships only the delta, and an unchanged tree is not sent.
pub const DEFAULT_SYNC_INTERVAL_SECS: u64 = 60;

fn refresh_interval(raw: Option<&str>) -> Duration {
    Duration::from_secs(
        raw.and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_REFRESH_INTERVAL_SECS),
    )
}

/// `0` yields `None`; unset or garbage yields the default.
fn sync_interval(raw: Option<&str>) -> Option<Duration> {
    match raw.map(str::parse::<u64>) {
        Some(Ok(0)) => None,
        Some(Ok(secs)) => Some(Duration::from_secs(secs)),
        _ => Some(Duration::from_secs(DEFAULT_SYNC_INTERVAL_SECS)),
    }
}

/// What a `SANDBOX_ONESHOT=sync` rescue pod reads: no backend connection, so
/// none of [`Config`]'s required fields.
pub struct RescueConfig {
    pub task_id: String,
    pub workspace_dir: String,
    pub put_url: String,
    pub http: SandboxHttpConfig,
}

impl RescueConfig {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            task_id: env::var("TASK_ID").map_err(|_| "SANDBOX_ONESHOT=sync requires TASK_ID")?,
            workspace_dir: env::var("WORKSPACE_DIR").unwrap_or_else(|_| "/workspace".to_string()),
            put_url: env::var("WORKSPACE_SYNC_PUT_URL")
                .map_err(|_| "SANDBOX_ONESHOT=sync requires WORKSPACE_SYNC_PUT_URL")?,
            http: SandboxHttpConfig::from_vars(|key| env::var(key).ok()),
        })
    }
}

/// Where uploads go when the backend supplied no `UPLOAD_DIR`.
///
/// Inside the workspace (so `workspace_sync` restores it) but a sibling of the
/// repo checkouts, so it stays out of `git status`.
fn default_upload_dir(workspace_dir: &str) -> String {
    format!("{}/uploads", workspace_dir.trim_end_matches('/'))
}

fn env_flag_value_enabled(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true")
}

fn env_flag_enabled(name: &str) -> bool {
    env::var(name).is_ok_and(|v| env_flag_value_enabled(&v))
}

/// Read a boolean env var as a tri-state override: `None` when unset or empty,
/// otherwise whether the value is truthy per [`env_flag_value_enabled`].
///
/// Distinct from [`env_flag_enabled`], which folds "unset" into `false`.
fn env_flag_override(name: &str) -> Option<bool> {
    match env::var(name) {
        Ok(value) if !value.is_empty() => Some(env_flag_value_enabled(&value)),
        _ => None,
    }
}

impl Config {
    /// Where the task owner's plugins are fetched to: `$WORKSPACE_DIR/.plugins`.
    ///
    /// Under the workspace so `workspace_sync` restores them after a pod
    /// replacement; hidden so `files::git::discover_git_repos` does not list
    /// them beside the repo checkouts.
    pub fn plugins_dir(&self) -> PathBuf {
        Path::new(&self.workspace_dir).join(".plugins")
    }

    /// Absolute checkout directories, primary first, falling back to a nonempty `repo_dir`.
    pub fn repo_dirs(&self) -> Vec<String> {
        if self.repos.is_empty() {
            if self.repo_dir.is_empty() {
                Vec::new()
            } else {
                vec![self.repo_dir.clone()]
            }
        } else {
            self.repos
                .iter()
                .map(|repo| repo.dir(&self.workspace_dir))
                .collect()
        }
    }

    /// The origin a proxied checkout clones from — `{scheme}://{host}[:{port}]`
    /// of [`Self::backend_api_url`], path dropped. Git matches
    /// `credential.<url>.helper` against this, so it must carry the scheme.
    pub fn backend_api_origin(&self) -> Option<String> {
        let (scheme, authority) = scheme_and_authority(self.backend_api_url.as_deref()?)?;
        Some(format!("{scheme}://{authority}"))
    }

    /// Whether `repo` clones from the backend proxy, which authenticates the
    /// pod's task token and so must be handed no provider token.
    pub fn clones_through_proxy(&self, repo: &RepoCheckout) -> bool {
        self.backend_api_url
            .as_deref()
            .is_some_and(|base| proxy_clone_repository_id(base, &repo.clone_url).is_some())
    }

    /// The `host[:port]` git sends as its `host=` field when it asks for a
    /// proxied checkout's credential.
    pub fn backend_api_host(&self) -> Option<String> {
        let (_, authority) = scheme_and_authority(self.backend_api_url.as_deref()?)?;
        Some(authority.to_string())
    }

    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self, String> {
        let parsed = parse_repo_config();

        let home_dir = env::var("HOME").unwrap_or_else(|_| "/home/moatless".to_string());

        // Mirrors the Claude CLI: `CLAUDE_CONFIG_DIR`, then its `CLAUDE_HOME`
        // alias, then `{home_dir}/.claude`.
        let claude_config_dir = env::var("CLAUDE_CONFIG_DIR")
            .or_else(|_| env::var("CLAUDE_HOME"))
            .unwrap_or_else(|_| format!("{home_dir}/.claude"));
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
        let git_provider = env::var("GIT_PROVIDER").unwrap_or_default();
        let backend_api_url = env::var("BACKEND_API_URL").ok();
        let (repos, github_upstreams) =
            load_repo_checkouts(git_clone_url.as_deref(), backend_api_url.as_deref())?;
        let repo_dir = primary_checkout(&repos)
            .map_or_else(|| workspace_dir.clone(), |repo| repo.dir(&workspace_dir));
        let upload_dir =
            env::var("UPLOAD_DIR").unwrap_or_else(|_| default_upload_dir(&workspace_dir));

        Ok(Self {
            port: env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8001),

            grpc_port: env::var("GRPC_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50052),

            #[allow(clippy::expect_used)] // required env var, crash at startup if missing
            sandbox_id: env::var("SANDBOX_ID")
                .or_else(|_| env::var("TASK_ID"))
                .expect("SANDBOX_ID or TASK_ID is required"),

            #[allow(clippy::expect_used)] // initial member env var, crash at startup if missing
            task_id: env::var("TASK_ID").expect("TASK_ID is required"),

            agent_profile: resolve_agent_profile_env(),

            workspace_dir,
            repo_dir,

            data_dir: env::var("DATA_DIR")
                .unwrap_or_else(|_| "/opt/moatless/data".to_string()),

            // Sandboxes sharing one host (`SANDBOX_PROVIDER=local`) each need
            // their own credential socket directory.
            runtime_dir: env::var("RUNTIME_DIR").unwrap_or_else(|_| RUNTIME_DIR.to_string()),

            upload_dir,

            log_level: env::var("LOG_LEVEL")
                .unwrap_or_else(|_| "info".to_string()),

            #[allow(clippy::expect_used)] // required env var, crash at startup if missing
            backend_grpc_url: env::var("BACKEND_GRPC_URL")
                .expect("BACKEND_GRPC_URL is required"),

            backend_api_url,

            user_auth_token: env::var("USER_AUTH_TOKEN").ok(),

            bootstrap_token: env::var("SANDBOX_BOOTSTRAP_TOKEN").ok(),

            // `SANDBOX_FRONTLOAD_REPO_INSTRUCTIONS` outranks REPO_CONFIG. It is
            // the lever a repo can actually reach today: the provisioning path
            // rebuilds REPO_CONFIG from typed fields, but per-repo `envVars`
            // already flow verbatim into this container.
            agent_config: AgentConfig {
                frontload_instructions: env_flag_override("SANDBOX_FRONTLOAD_REPO_INSTRUCTIONS")
                    .or(parsed.agent_config.frontload_instructions),
                ..parsed.agent_config
            },

            claude_cli_path: env::var("SANDBOX_CLAUDE_EXECUTABLE_PATH")
                .unwrap_or_else(|_| "/usr/local/bin/claude".to_string()),
            codex_cli_path: env::var("SANDBOX_CODEX_EXECUTABLE_PATH")
                .unwrap_or_else(|_| "/usr/local/bin/codex".to_string()),

            frontend_url: env::var("FRONTEND_URL").ok(),
            default_model: env::var("DEFAULT_CLAUDE_CODE_MODEL").ok(),
            openai_base_url: env::var("OPENAI_BASE_URL")
                .ok()
                .filter(|u| !u.trim().is_empty()),
            use_tmux: env_flag_enabled("SANDBOX_USE_TMUX"),
            command_default_timeout_secs: env::var("SANDBOX_COMMAND_DEFAULT_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECS),
            permission_mode: env::var("SANDBOX_PERMISSION_MODE")
                .ok()
                .and_then(|v| parse_permission_mode(&v))
                .unwrap_or(PermissionMode::BypassPermissions),
            workspace_sync_enabled: env_flag_enabled("WORKSPACE_SYNC_ENABLED"),
            workspace_restore_if_empty: env_flag_enabled("WORKSPACE_RESTORE_IF_EMPTY"),
            headless: env_flag_enabled("SANDBOX_HEADLESS"),
            credential_refresh_interval: refresh_interval(
                env::var("CREDENTIAL_REFRESH_INTERVAL_SECS").ok().as_deref(),
            ),
            transcript_sync_interval: sync_interval(
                env::var("TRANSCRIPT_SYNC_INTERVAL_SECS").ok().as_deref(),
            ),
            workspace_sync_interval: sync_interval(
                env::var("WORKSPACE_SYNC_INTERVAL_SECS").ok().as_deref(),
            ),
            workspace_limit: env::var(WORKSPACE_LIMIT_ENV)
                .ok()
                .map(|raw| raw.trim().to_string())
                .filter(|raw| !raw.is_empty()),
            shell: env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),

            home_dir,
            codex_home_dir,
            claude_config_dir,
            git_clone_url,
            git_branch,
            git_provider,
            repos,
            github_upstreams,
            repo_config_raw: env::var("REPO_CONFIG").ok(),
            setup_commands: parsed.setup_commands,
            plugins: parsed.plugins,
            language_servers: parsed.language_servers,
            search_path: env::var_os("PATH"),
            http: SandboxHttpConfig::from_vars(|key| env::var(key).ok()),
        })
    }

    /// Filesystem path where the long-lived user auth token is persisted after
    /// `GetCredentials` fetches it.
    ///
    /// Lives in [`RUNTIME_DIR`], neither archived into the workspace snapshot
    /// nor mounted into the server sidecars.
    pub fn persisted_user_token_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.runtime_dir).join(".user_auth_token")
    }

    /// Every token the sandbox can present on its first gRPC calls, in the
    /// order init should try them, without repeats.
    ///
    /// No single store is reliably the freshest, so init works down this list
    /// on an `Unauthenticated` rejection.
    pub fn initial_auth_token_candidates(&self) -> Vec<String> {
        let persisted = std::fs::read_to_string(self.persisted_user_token_path())
            .ok()
            .map(|token| token.trim().to_string());

        let mut candidates = Vec::new();
        for token in [
            persisted,
            self.user_auth_token.clone(),
            self.bootstrap_token.clone(),
        ]
        .into_iter()
        .flatten()
        {
            if !token.is_empty() && !candidates.contains(&token) {
                candidates.push(token);
            }
        }
        candidates
    }

    /// The token the sandbox presents before `GetCredentials` delivers a fresh
    /// long-lived one — the first of [`Self::initial_auth_token_candidates`].
    pub fn resolve_initial_auth_token(&self) -> Option<String> {
        self.initial_auth_token_candidates().into_iter().next()
    }
}

/// Test-only helpers for constructing a [`Config`] without env vars.
#[cfg(test)]
pub(crate) mod test_support {
    use super::{
        AgentConfig, Config, DEFAULT_REFRESH_INTERVAL_SECS, DEFAULT_SYNC_INTERVAL_SECS,
        PermissionMode,
    };
    use std::collections::HashMap;
    use std::time::Duration;

    /// A minimal [`Config`] with empty/default values, suitable for unit tests
    /// that only exercise a few fields. Callers override what they need.
    pub(crate) fn minimal_config() -> Config {
        Config {
            port: 8001,
            grpc_port: 50052,
            sandbox_id: "test-sandbox".to_string(),
            task_id: "test-task".to_string(),
            agent_profile: None,
            workspace_dir: "/tmp/workspace".to_string(),
            repo_dir: "/tmp/workspace/service".to_string(),
            data_dir: "/tmp/data".to_string(),
            runtime_dir: "/tmp/run".to_string(),
            upload_dir: "/tmp/uploads".to_string(),
            log_level: "info".to_string(),
            backend_grpc_url: "http://backend:50051".to_string(),
            backend_api_url: None,
            user_auth_token: None,
            bootstrap_token: None,
            agent_config: AgentConfig::default(),
            claude_cli_path: "/usr/local/bin/claude".to_string(),
            codex_cli_path: "/usr/local/bin/codex".to_string(),
            frontend_url: None,
            default_model: None,
            openai_base_url: None,
            use_tmux: false,
            command_default_timeout_secs: crate::config::DEFAULT_COMMAND_TIMEOUT_SECS,
            permission_mode: PermissionMode::BypassPermissions,
            workspace_sync_enabled: false,
            workspace_restore_if_empty: false,
            headless: false,
            credential_refresh_interval: Duration::from_secs(DEFAULT_REFRESH_INTERVAL_SECS),
            transcript_sync_interval: Some(Duration::from_secs(DEFAULT_SYNC_INTERVAL_SECS)),
            workspace_sync_interval: Some(Duration::from_secs(DEFAULT_SYNC_INTERVAL_SECS)),
            workspace_limit: None,
            shell: "/bin/bash".to_string(),
            home_dir: "/tmp/home".to_string(),
            codex_home_dir: "/tmp/home/.codex".to_string(),
            claude_config_dir: "/tmp/home/.claude".to_string(),
            git_clone_url: None,
            git_branch: None,
            git_provider: "github".to_string(),
            repos: Vec::new(),
            github_upstreams: HashMap::new(),
            repo_config_raw: None,
            setup_commands: Vec::new(),
            plugins: Vec::new(),
            language_servers: Vec::new(),
            search_path: None,
            http: super::SandboxHttpConfig::default(),
        }
    }
}

/// Parses `REPO_CONFIG` into server, agent, and setup-command settings, folding
/// the two `frontloadInstructions` spellings into one [`AgentConfig`].
///
/// The top-level spelling wins over the nested `agentConfig` one; `None` when
/// both are unset.
fn resolve_agent_config(
    agent_config: Option<RepoAgentConfig>,
    frontload_instructions: Option<bool>,
) -> AgentConfig {
    match agent_config {
        Some(ac) => AgentConfig {
            timeout: ac.timeout,
            model: ac.model,
            system_prompt: ac.system_prompt,
            frontload_instructions: frontload_instructions.or(ac.frontload_instructions),
        },
        None => AgentConfig {
            frontload_instructions,
            ..AgentConfig::default()
        },
    }
}

fn parse_repo_config() -> ParsedRepoConfig {
    let raw = match env::var("REPO_CONFIG") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            return ParsedRepoConfig {
                agent_config: AgentConfig::default(),
                setup_commands: Vec::new(),
                plugins: Vec::new(),
                language_servers: Vec::new(),
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
                language_servers: Vec::new(),
            };
        }
    };

    let agent_config =
        resolve_agent_config(repo_config.agent_config, repo_config.frontload_instructions);

    let setup_commands = repo_config
        .code_execution
        .map(|ce| ce.setup_commands)
        .unwrap_or_default();

    let plugins = repo_config.plugins.unwrap_or_default();
    let language_servers = parse_language_servers(repo_config.language_servers);

    ParsedRepoConfig {
        agent_config,
        setup_commands,
        plugins,
        language_servers,
    }
}

/// The known, distinct ids in `REPO_CONFIG.languageServers`, in order.
fn parse_language_servers(ids: Option<Vec<String>>) -> Vec<LanguageServer> {
    let mut servers = Vec::new();
    for id in ids.unwrap_or_default() {
        match id.parse::<LanguageServer>() {
            Ok(server) if !servers.contains(&server) => servers.push(server),
            Ok(_) => {}
            Err(_) => tracing::warn!(id = %id, "Ignoring unknown language server"),
        }
    }
    servers
}

#[cfg(test)]
mod tests {
    use super::test_support::minimal_config;
    use super::{
        PermissionMode, RepoConfigFile, default_upload_dir, env_flag_override,
        env_flag_value_enabled, is_safe_mount_name, parse_permission_mode, parse_repo_checkouts,
        primary_checkout, resolve_agent_config, route_through_proxy,
    };
    use moatless_workspace_types::workspace::{LanguageServer, RepoCheckout};
    use std::collections::HashMap;
    use std::time::Duration;

    #[test]
    fn sync_intervals_default_and_disable() {
        let default = Some(Duration::from_secs(super::DEFAULT_SYNC_INTERVAL_SECS));
        assert_eq!(super::sync_interval(None), default);
        assert_eq!(super::sync_interval(Some("0")), None);
        assert_eq!(
            super::sync_interval(Some("30")),
            Some(Duration::from_secs(30))
        );
        assert_eq!(super::sync_interval(Some("garbage")), default);
        assert_eq!(super::refresh_interval(Some("0")), Duration::from_mins(15));
        assert_eq!(super::refresh_interval(Some("5")), Duration::from_secs(5));
    }

    /// Unset and empty are "no opinion", so a default-on setting falls through
    /// to the next source instead of being switched off by absence.
    #[test]
    fn env_flag_override_is_none_when_unset_or_empty() {
        // A name no other test touches, so the parallel test runner can't race it.
        const PROBE: &str = "SANDBOX_TEST_FRONTLOAD_OVERRIDE_PROBE";
        std::env::remove_var(PROBE);
        assert_eq!(env_flag_override(PROBE), None);

        std::env::set_var(PROBE, "");
        assert_eq!(env_flag_override(PROBE), None);

        std::env::set_var(PROBE, "false");
        assert_eq!(env_flag_override(PROBE), Some(false));

        std::env::set_var(PROBE, "true");
        assert_eq!(env_flag_override(PROBE), Some(true));

        std::env::set_var(PROBE, "1");
        assert_eq!(env_flag_override(PROBE), Some(true));

        // Anything not truthy reads as an explicit off, not as "unset".
        std::env::set_var(PROBE, "0");
        assert_eq!(env_flag_override(PROBE), Some(false));

        std::env::remove_var(PROBE);
    }

    /// An id this build does not know drops alone rather than discarding the
    /// whole `REPO_CONFIG`, and a repeated id starts one server.
    #[test]
    fn language_servers_skip_unknown_and_repeated_ids() {
        let parsed: RepoConfigFile =
            serde_json::from_str(r#"{"languageServers":["rust","cobol","rust","typescript"]}"#)
                .expect("valid REPO_CONFIG");
        assert_eq!(
            super::parse_language_servers(parsed.language_servers),
            [LanguageServer::Rust, LanguageServer::Typescript]
        );
    }

    fn agent_config_from_json(raw: &str) -> super::AgentConfig {
        let parsed: RepoConfigFile = serde_json::from_str(raw).expect("valid REPO_CONFIG");
        resolve_agent_config(parsed.agent_config, parsed.frontload_instructions)
    }

    /// A `REPO_CONFIG` that says nothing about frontloading leaves it enabled,
    /// so a repo gets its instructions in context without opting in.
    #[test]
    fn frontloading_is_enabled_when_repo_config_is_silent() {
        assert!(agent_config_from_json("{}").frontload_instructions_enabled());
        assert!(
            agent_config_from_json(r#"{"agentConfig":{"model":"opus"}}"#)
                .frontload_instructions_enabled()
        );
    }

    /// The top-level key is the shape the provisioning path emits.
    #[test]
    fn top_level_frontload_instructions_disables_frontloading() {
        let resolved = agent_config_from_json(r#"{"frontloadInstructions":false}"#);
        assert!(!resolved.frontload_instructions_enabled());
    }

    /// The nested spelling still works for `REPO_CONFIG` supplied verbatim.
    #[test]
    fn nested_frontload_instructions_disables_frontloading() {
        let resolved = agent_config_from_json(r#"{"agentConfig":{"frontloadInstructions":false}}"#);
        assert!(!resolved.frontload_instructions_enabled());
    }

    /// Top level wins when the two disagree.
    #[test]
    fn top_level_frontload_instructions_overrides_the_nested_spelling() {
        let resolved = agent_config_from_json(
            r#"{"frontloadInstructions":true,"agentConfig":{"frontloadInstructions":false}}"#,
        );
        assert!(resolved.frontload_instructions_enabled());

        let resolved = agent_config_from_json(
            r#"{"frontloadInstructions":false,"agentConfig":{"frontloadInstructions":true}}"#,
        );
        assert!(!resolved.frontload_instructions_enabled());
    }

    /// Resolving frontloading must not disturb the other agent settings.
    #[test]
    fn resolving_frontloading_preserves_the_other_agent_settings() {
        let resolved = agent_config_from_json(
            r#"{"frontloadInstructions":false,"agentConfig":{"timeout":42,"model":"opus","systemPrompt":"hi"}}"#,
        );
        assert_eq!(resolved.timeout, Some(42));
        assert_eq!(resolved.model.as_deref(), Some("opus"));
        assert_eq!(resolved.system_prompt.as_deref(), Some("hi"));
        assert!(!resolved.frontload_instructions_enabled());
    }

    /// Point the persisted-token dir at an empty temp dir so resolution falls
    /// through to the env-supplied tokens deterministically.
    fn config_without_persisted_token() -> (super::Config, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = minimal_config();
        config.workspace_dir = tmp.path().display().to_string();
        config.data_dir = tmp.path().display().to_string();
        // Per-test dir, so one test's persisted token cannot be read by another.
        config.runtime_dir = tmp.path().display().to_string();
        (config, tmp)
    }

    #[test]
    fn default_upload_dir_sits_inside_the_workspace() {
        assert_eq!(
            default_upload_dir("/opt/moatless/workspace"),
            "/opt/moatless/workspace/uploads"
        );
    }

    #[test]
    fn default_upload_dir_tolerates_a_trailing_slash() {
        assert_eq!(default_upload_dir("/ws/"), "/ws/uploads");
    }

    #[test]
    fn env_flag_value_enabled_accepts_only_truthy_values() {
        assert!(env_flag_value_enabled("true"));
        assert!(env_flag_value_enabled("TRUE"));
        assert!(env_flag_value_enabled("1"));
        assert!(!env_flag_value_enabled(""));
        assert!(!env_flag_value_enabled("false"));
        assert!(!env_flag_value_enabled("0"));
    }

    fn checkout(mount_name: &str, is_primary: bool) -> super::RepoCheckout {
        super::RepoCheckout {
            uses_ssh: false,
            clone_url: "https://example.com/repo.git".into(),
            repository_id: String::new(),
            branch: None,
            provider: "github".into(),
            mount_name: mount_name.into(),
            is_primary,
        }
    }

    #[test]
    fn repo_dirs_lists_every_checkout_under_the_workspace() {
        let mut config = minimal_config();
        config.workspace_dir = "/ws".into();
        config.repo_dir = "/ws/api".into();
        config.repos = vec![checkout("api", true), checkout("web", false)];
        assert_eq!(config.repo_dirs(), vec!["/ws/api", "/ws/web"]);
    }

    #[test]
    fn repo_dirs_falls_back_to_repo_dir_without_workspace_repos() {
        let mut config = minimal_config();
        config.repo_dir = "/ws/only".into();
        config.repos = Vec::new();
        assert_eq!(config.repo_dirs(), vec!["/ws/only"]);
    }

    #[test]
    fn repo_dirs_empty_when_nothing_checked_out() {
        let mut config = minimal_config();
        config.repo_dir = String::new();
        config.repos = Vec::new();
        assert!(config.repo_dirs().is_empty());
    }

    #[test]
    fn resolve_initial_auth_token_prefers_legacy_over_bootstrap() {
        // Prefer the long-lived env token for reliability across reschedules;
        // bootstrap stays as the fallback.
        let (mut config, _tmp) = config_without_persisted_token();
        config.bootstrap_token = Some("boot".to_string());
        config.user_auth_token = Some("legacy".to_string());
        assert_eq!(
            config.resolve_initial_auth_token(),
            Some("legacy".to_string())
        );
    }

    #[test]
    fn resolve_initial_auth_token_uses_bootstrap_when_legacy_absent() {
        // Bootstrap is the fallback when no long-lived env token is present.
        let (mut config, _tmp) = config_without_persisted_token();
        config.bootstrap_token = Some("boot".to_string());
        config.user_auth_token = None;
        assert_eq!(
            config.resolve_initial_auth_token(),
            Some("boot".to_string())
        );
    }

    #[test]
    fn resolve_initial_auth_token_prefers_persisted_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = minimal_config();
        config.workspace_dir = tmp.path().join("workspace").display().to_string();
        config.data_dir = tmp.path().join("data").display().to_string();
        config.runtime_dir = tmp.path().join("run").display().to_string();
        config.bootstrap_token = Some("boot".to_string());
        config.user_auth_token = Some("legacy".to_string());
        let path = config.persisted_user_token_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "persisted-tok\n").unwrap();
        assert_eq!(
            config.resolve_initial_auth_token(),
            Some("persisted-tok".to_string())
        );
    }

    #[test]
    fn initial_auth_token_candidates_offers_every_token_once() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = minimal_config();
        config.workspace_dir = tmp.path().join("workspace").display().to_string();
        config.data_dir = tmp.path().join("data").display().to_string();
        config.runtime_dir = tmp.path().join("run").display().to_string();
        // The env and bootstrap tokens are minted together and can be equal;
        // presenting the same rejected token twice would waste a retry.
        config.user_auth_token = Some("same".to_string());
        config.bootstrap_token = Some("same".to_string());
        let path = config.persisted_user_token_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "persisted\n").unwrap();

        assert_eq!(
            config.initial_auth_token_candidates(),
            vec!["persisted".to_string(), "same".to_string()]
        );
    }

    #[test]
    fn initial_auth_token_candidates_skips_an_empty_persisted_file() {
        let (mut config, _tmp) = config_without_persisted_token();
        std::fs::write(config.persisted_user_token_path(), "  \n").unwrap();
        config.bootstrap_token = Some("boot".to_string());

        assert_eq!(config.initial_auth_token_candidates(), vec!["boot"]);
    }

    #[test]
    fn persisted_user_token_path_stays_off_the_workspace_tree() {
        // The workspace tree is archived into the task snapshot and restored into
        // replacement pods; a token kept there would outlive its pod.
        let mut config = minimal_config();
        config.workspace_dir = "/opt/moatless/workspace".to_string();
        config.data_dir = "/opt/moatless/data".to_string();
        assert!(
            !config
                .persisted_user_token_path()
                .starts_with(&config.workspace_dir)
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
    fn url_only_configuration_requires_reprovisioning() {
        assert!(
            parse_repo_checkouts(None, Some("https://github.com/owner/service.git"))
                .unwrap_err()
                .contains("reprovision")
        );
    }

    #[test]
    fn no_clone_url_at_all_yields_no_checkouts() {
        assert!(parse_repo_checkouts(None, None).unwrap().is_empty());
        assert!(parse_repo_checkouts(Some("[]"), None).unwrap().is_empty());
    }

    #[test]
    fn workspace_repos_env_supplies_every_checkout() {
        let raw = r#"[
            {"usesSsh":false,"cloneUrl":"https://github.com/acme/api.git","branch":"main","provider":"github","mountName":"api","isPrimary":true},
            {"usesSsh":false,"cloneUrl":"https://gitlab.com/acme/shared.git","provider":"gitlab","mountName":"lib"}
        ]"#;
        let repos =
            parse_repo_checkouts(Some(raw), Some("https://github.com/acme/api.git")).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(
            primary_checkout(&repos)
                .unwrap()
                .dir("/opt/moatless/workspace"),
            "/opt/moatless/workspace/api"
        );
        assert_eq!(repos[1].provider, "gitlab");
        assert_eq!(repos[1].branch, None);
        assert!(!repos[1].is_primary);
    }

    /// Mount names are joined onto the workspace dir, so an entry that escapes
    /// it is dropped rather than repaired — the backend already sanitizes.
    #[test]
    fn checkouts_with_unusable_mount_names_are_dropped() {
        let raw = r#"[
            {"usesSsh":false,"cloneUrl":"https://github.com/acme/api.git","provider":"github","mountName":"api","isPrimary":true},
            {"usesSsh":false,"cloneUrl":"https://github.com/acme/evil.git","provider":"github","mountName":"../escape"},
            {"usesSsh":false,"cloneUrl":"","provider":"github","mountName":"empty"}
        ]"#;
        let repos = parse_repo_checkouts(Some(raw), None).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].mount_name, "api");
    }

    #[test]
    fn malformed_or_unclassified_checkouts_are_rejected() {
        assert!(parse_repo_checkouts(Some("not json"), None).is_err());
        assert!(parse_repo_checkouts(Some(r#"[{"cloneUrl":"git@github.com:acme/repo.git","provider":"github","mountName":"repo"}]"#), None).is_err());
    }

    fn routed(
        raw: &str,
        backend_api_url: Option<&str>,
    ) -> (Vec<RepoCheckout>, HashMap<String, String>) {
        let mut repos = parse_repo_checkouts(Some(raw), None).unwrap();
        let upstreams = route_through_proxy(&mut repos, backend_api_url);
        (repos, upstreams)
    }

    /// A Forgejo token never reaches the pod, so its checkout clones from the
    /// backend even when it was declared over SSH.
    #[test]
    fn a_forgejo_checkout_clones_through_the_backend_proxy() {
        let raw = r#"[
            {"usesSsh":true,"cloneUrl":"git@forge.example:acme/api.git","repositoryId":"repo_1","provider":"forgejo","mountName":"api","isPrimary":true},
            {"usesSsh":false,"cloneUrl":"https://gitlab.com/acme/lib.git","repositoryId":"repo_3","provider":"gitlab","mountName":"lib"}
        ]"#;
        let (repos, upstreams) = routed(raw, Some("http://backend:8001/"));
        assert_eq!(repos[0].clone_url, "http://backend:8001/git/repo_1");
        assert!(!repos[0].uses_ssh);
        assert_eq!(repos[1].clone_url, "https://gitlab.com/acme/lib.git");
        assert!(upstreams.is_empty(), "{upstreams:?}");
    }

    /// An HTTPS GitHub checkout clones from the backend too, and its GitHub URL
    /// is kept for the remote `gh` resolves the repository from.
    #[test]
    fn a_github_checkout_with_a_repository_id_clones_through_the_backend_proxy() {
        let raw = r#"[
            {"usesSsh":false,"cloneUrl":"https://github.com/acme/web.git","repositoryId":"repo_2","provider":"github","mountName":"web","isPrimary":true}
        ]"#;
        let (repos, upstreams) = routed(raw, Some("http://backend:8001"));
        assert_eq!(repos[0].clone_url, "http://backend:8001/git/repo_2");
        assert_eq!(
            upstreams.get("web").map(String::as_str),
            Some("https://github.com/acme/web.git")
        );
    }

    /// No registry row, or SSH keys the user chose, leaves github.com in place.
    #[test]
    fn a_github_checkout_without_an_id_or_over_ssh_keeps_its_remote() {
        let raw = r#"[
            {"usesSsh":false,"cloneUrl":"https://github.com/acme/web.git","provider":"github","mountName":"web","isPrimary":true},
            {"usesSsh":true,"cloneUrl":"git@github.com:acme/api.git","repositoryId":"repo_1","provider":"github","mountName":"api"}
        ]"#;
        let (repos, upstreams) = routed(raw, Some("http://backend:8001"));
        assert_eq!(repos[0].clone_url, "https://github.com/acme/web.git");
        assert_eq!(repos[1].clone_url, "git@github.com:acme/api.git");
        assert!(repos[1].uses_ssh);
        assert!(upstreams.is_empty(), "{upstreams:?}");
    }

    #[test]
    fn only_a_backend_proxy_url_counts_as_cloning_through_the_proxy() {
        let mut config = minimal_config();
        config.backend_api_url = Some("http://backend:8001".to_string());
        let raw = r#"[
            {"usesSsh":false,"cloneUrl":"https://github.com/acme/web.git","repositoryId":"repo_2","provider":"github","mountName":"web","isPrimary":true},
            {"usesSsh":false,"cloneUrl":"https://gitlab.com/acme/lib.git","repositoryId":"repo_3","provider":"gitlab","mountName":"lib"}
        ]"#;
        let (repos, _) = routed(raw, config.backend_api_url.as_deref());
        assert!(config.clones_through_proxy(&repos[0]));
        assert!(!config.clones_through_proxy(&repos[1]));
    }

    /// With nothing to address the backend by, the upstream URL stays: the
    /// clone then fails on the credential the pod does not hold, which is a
    /// report.
    #[test]
    fn a_forgejo_checkout_the_proxy_cannot_be_addressed_for_keeps_its_remote() {
        let raw = r#"[{"usesSsh":false,"cloneUrl":"https://forge.example/acme/api.git","repositoryId":"repo_1","provider":"forgejo","mountName":"api","isPrimary":true}]"#;
        assert_eq!(
            routed(raw, None).0[0].clone_url,
            "https://forge.example/acme/api.git"
        );

        let no_id = r#"[{"usesSsh":false,"cloneUrl":"https://forge.example/acme/api.git","provider":"forgejo","mountName":"api","isPrimary":true}]"#;
        assert_eq!(
            routed(no_id, Some("http://backend:8001")).0[0].clone_url,
            "https://forge.example/acme/api.git"
        );
    }

    #[test]
    fn the_backend_origin_drops_the_path_and_keeps_the_port() {
        let mut config = minimal_config();
        config.backend_api_url = Some("http://backend.svc:8001/api/v1".to_string());
        assert_eq!(
            config.backend_api_origin().as_deref(),
            Some("http://backend.svc:8001")
        );
        assert_eq!(
            config.backend_api_host().as_deref(),
            Some("backend.svc:8001")
        );

        config.backend_api_url = None;
        assert_eq!(config.backend_api_origin(), None);
        assert_eq!(config.backend_api_host(), None);
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
        let raw = r#"[{"usesSsh":false,"cloneUrl":"https://github.com/acme/api.git","provider":"github","mountName":"api"}]"#;
        let repos = parse_repo_checkouts(Some(raw), None).unwrap();
        assert_eq!(primary_checkout(&repos).unwrap().mount_name, "api");
        assert!(primary_checkout(&[]).is_none());
    }
}
