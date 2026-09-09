//! Configuration loaded from environment variables.
//!
//! The sandbox runs inside a sandbox pod and receives its configuration
//! via environment variables set by the orchestrator.

use std::env;

use claude_agent_sdk::PermissionMode;
use moatless_types::plugin::PluginDelivery;
use moatless_types::workspace::{
    PROXIED_PROVIDER, RepoCheckout, WORKSPACE_REPOS_ENV, derive_mount_name, proxy_clone_url,
};
use serde::Deserialize;

/// Sandbox-private runtime directory.
///
/// Its volume is mounted into the sandbox container alone — not into the server
/// sidecars, which run repo code — and it sits outside `workspace_dir`, so
/// nothing here reaches the workspace snapshot. Credential material that must
/// not travel lives here: the credential broker socket and the persisted user
/// auth token.
///
/// Fixed rather than configurable because `docker/moatless-git-cred.sh`
/// hardcodes the broker socket beneath it.
pub const RUNTIME_DIR: &str = "/run/moatless";

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
    /// Whether to inline repo root instruction files into the system prompt
    /// rather than only listing their paths.
    ///
    /// Defaults to on when unset: a path the agent is told to read is not a
    /// guarantee it reads one, so the conventions start in context. A repo that
    /// would rather spend those tokens elsewhere — a large instruction file, or
    /// one whose rules only matter for a slice of the tree — turns it off and
    /// gets the path listing back.
    ///
    /// Resolved from, in order: the `SANDBOX_FRONTLOAD_REPO_INSTRUCTIONS` env
    /// var, then `REPO_CONFIG` (`frontloadInstructions`, top level or nested under
    /// `agentConfig`), then this default. The env var is the one a repo can set
    /// per-repo today, via `envVars` in its run-config: those reach this
    /// container verbatim, whereas `REPO_CONFIG` is rebuilt upstream from typed
    /// fields that do not yet carry this flag.
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

/// Server info parsed from `REPO_CONFIG`, used for system prompt.
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

/// Partial `REPO_CONFIG` JSON structure — we only parse the fields we need.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoConfigFile {
    #[serde(default)]
    agent_config: Option<RepoAgentConfig>,
    /// `frontloadInstructions` at the top level — the shape the provisioning
    /// path emits, which rebuilds this JSON from typed fields and writes flat
    /// keys (as it does for `systemPrompt`) rather than an `agentConfig` object.
    /// Takes precedence over the nested spelling.
    #[serde(default)]
    frontload_instructions: Option<bool>,
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
    /// Sandbox ID (required for new pods; falls back to `TASK_ID` for older local fixtures).
    pub sandbox_id: String,
    /// Initial member Task ID.
    pub task_id: String,
    /// The Task's selected Agent harness, fixed for the Sandbox lifetime and
    /// delivered at construction via the `AGENT_TYPE` env (the fixed-harness decision). `None`
    /// means the pod has no valid harness selection and is surfaced as an invalid
    /// configuration by the agent lifecycle owner. When set it is validated at
    /// load, so an unsupported value fails startup instead of silently running
    /// Claude.
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
    /// Sandbox-private runtime directory ([`RUNTIME_DIR`]), for credential
    /// material that must not reach repo code or the workspace snapshot.
    pub runtime_dir: String,
    /// Where a message's attached uploads are written before the agent runs
    /// (env `UPLOAD_DIR`, set by the backend from its `file_mount_path` so the
    /// prompt's "Saved to:" line and this agree). Defaults under
    /// [`Self::workspace_dir`], which is what makes an upload survive a restart:
    /// the workspace snapshot covers loose directories, so a restored pod finds
    /// the bytes already on disk and skips the re-fetch.
    pub upload_dir: String,
    /// Log level
    pub log_level: String,
    /// Backend gRPC URL (e.g. "<http://backend:50051>") — required
    pub backend_grpc_url: String,
    /// Backend API URL for HTTP fallback calls.
    pub backend_api_url: Option<String>,
    /// Long-lived user auth token from `USER_AUTH_TOKEN`, when present. K8s pods
    /// normally receive the long-lived token over `GetCredentials`; Docker and
    /// compatibility paths may still provide this env.
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
    /// Server configurations parsed from `REPO_CONFIG`
    pub servers: Vec<ServerInfo>,
    /// Frontend URL for constructing task URLs (e.g., "<https://moatless.io>")
    pub frontend_url: Option<String>,
    /// Global default Claude model (env: `DEFAULT_CLAUDE_CODE_MODEL`), used when neither request nor repo config set one
    pub default_model: Option<String>,
    /// Global default Codex model (env: `DEFAULT_CODEX_MODEL`), used when neither request nor repo config set one.
    /// Falls through to [`crate::agent::codex_app_server::resolve_codex_model`] at call time if unset.
    pub default_codex_model: Option<String>,
    /// Base URL of an OpenAI-compatible proxy to route Codex through (env:
    /// `OPENAI_BASE_URL`), including the `/v1` suffix.
    ///
    /// Codex honours no base-URL environment variable of its own, so this is
    /// rendered into `config.toml` as a custom model provider by
    /// [`crate::init::codex_config`]. Setting it also suppresses the
    /// `auth.json` write and its watcher: the proxy key is delivered by
    /// `env_key` instead, and mirroring it back would overwrite the user's
    /// stored Codex OAuth credential.
    pub openai_base_url: Option<String>,
    /// Use the tmux-based Claude Code SDK transport instead of subprocess/NDJSON (env: `SANDBOX_USE_TMUX`).
    pub use_tmux: bool,
    /// Deadline applied to a `moat cmd` command that declares none (env:
    /// `SANDBOX_COMMAND_DEFAULT_TIMEOUT_SECS`, default 4h). A command's own
    /// value overrides it in either direction, including past this default.
    pub command_default_timeout_secs: u64,
    /// Permission mode for the Claude CLI in normal (non-plan) mode (env:
    /// `SANDBOX_PERMISSION_MODE`). Defaults to `bypassPermissions` so network
    /// tools like WebSearch/WebFetch aren't denied (they are not auto-approved
    /// by `acceptEdits` and the CLI never routes them through the stdio
    /// permission handler). Plan-mode requests always override this with
    /// `plan`. Ignored in tmux mode, which always uses
    /// `--dangerously-skip-permissions`. Accepts the CLI spellings: `default`,
    /// `acceptEdits`, `bypassPermissions`, `plan`, `delegate`, `dontAsk`.
    pub permission_mode: PermissionMode,
    /// Whether this sandbox snapshots its workspace to S3 (env:
    /// `WORKSPACE_SYNC_ENABLED`). Set by the backend when the repo's `s3`
    /// storage switch is on; when true, the sandbox restores the workspace from
    /// S3 on boot and syncs it back via backend-minted presigned URLs.
    pub workspace_sync_enabled: bool,
    /// How this sandbox stores its workspace (env:
    /// `WORKSPACE_SNAPSHOT_FORMAT`; `WORKSPACE_MIRROR_ENABLED` when unset).
    ///
    /// This value chooses what a sync **writes**. A restore does not consult it:
    /// `workspace_sync::restore_on_start` tries the repository, then the mirror,
    /// then the tar, whatever this says, so moving a task between formats on
    /// this image reverts nothing.
    ///
    /// **Rolling back to a pre-mirror image costs a turn of work once.** That
    /// image reads only `workspace.tar`, which stays frozen at whatever the last
    /// tar-format sync left — so the next boot reverts everything synced since,
    /// until the first turn writes a fresh tar.
    pub workspace_snapshot_format: WorkspaceSnapshotFormat,
    /// Whether the volume under `WORKSPACE_DIR` is durable (env:
    /// `WORKSPACE_RESTORE_IF_EMPTY`). Set by the backend when *both* storage
    /// switches are on, i.e. a PVC holds the live copy and S3 only holds a
    /// fallback snapshot.
    ///
    /// It makes the restore conditional: the snapshot is read only into a volume
    /// that holds nothing usable — freshly created, recreated after a close, or
    /// swapped for an emptyDir because the PVC would not attach. Overwriting a
    /// populated PVC with an older snapshot would lose work. It also turns off
    /// the periodic upload loop, whose whole purpose is to bound the loss when
    /// the only copy is one an involuntary kill destroys.
    pub workspace_restore_if_empty: bool,
    /// Whether the task runs headless (env: `SANDBOX_HEADLESS`), i.e. nobody is
    /// expected to be watching it. Set by the backend for tasks created without
    /// a browser (CLI, loops). Preview servers are already stripped from the
    /// provision plan for these; here it also removes the tools that would park
    /// the turn on an answer nobody is there to give (`AskUserQuestion`,
    /// `EnterPlanMode`, `ExitPlanMode`).
    pub headless: bool,

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
    /// Raw `REPO_CONFIG` JSON (for entrypoint compat)
    pub repo_config_raw: Option<String>,
    /// Setup commands from `REPO_CONFIG.codeExecution.setupCommands`
    pub setup_commands: Vec<String>,
    /// The plugins to clone, each with the skills to keep, from
    /// `REPO_CONFIG.plugins`
    pub plugins: Vec<PluginDelivery>,
}

/// Read the workspace's repository checkouts from `WORKSPACE_REPOS`.
///
/// Falls back to the single checkout described by the
/// `GIT_CLONE_URL`/`GIT_BRANCH`/`GIT_PROVIDER` trio when the env is absent or
/// unparseable — cloning the primary repo is strictly better than cloning
/// nothing. Returns an empty list when there is no repo to clone at all.
///
/// A mount name that is not a single safe path segment is rejected rather than
/// repaired: it is joined onto `workspace_dir`, and the backend already
/// sanitizes, so anything else means a payload we should not act on.
fn load_repo_checkouts(
    git_clone_url: Option<&str>,
    git_branch: Option<&str>,
    git_provider: &str,
    backend_api_url: Option<&str>,
) -> Vec<RepoCheckout> {
    parse_repo_checkouts(
        env::var(WORKSPACE_REPOS_ENV).ok().as_deref(),
        git_clone_url,
        git_branch,
        git_provider,
        backend_api_url,
    )
}

/// The env-free half of [`load_repo_checkouts`].
fn parse_repo_checkouts(
    raw: Option<&str>,
    git_clone_url: Option<&str>,
    git_branch: Option<&str>,
    git_provider: &str,
    backend_api_url: Option<&str>,
) -> Vec<RepoCheckout> {
    let legacy = || match git_clone_url {
        Some(url) => vec![RepoCheckout {
            clone_url: url.to_string(),
            branch: git_branch.map(str::to_string),
            // The legacy trio predates the multi-repo env and names no
            // repository, so a proxied clone is not reachable from it.
            repository_id: String::new(),
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

    let (mut usable, rejected): (Vec<_>, Vec<_>) = parsed
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
    for repo in &mut usable {
        if let Some(url) = proxied_clone_url(repo, backend_api_url) {
            repo.clone_url = url;
        }
    }
    usable
}

/// Split a URL into its scheme and its `host[:port]`, dropping any path.
fn scheme_and_authority(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split('/').next().filter(|a| !a.is_empty())?;
    Some((scheme, authority))
}

/// The backend-proxied remote to clone this checkout from, or `None` when it
/// clones from its own host.
///
/// Returns `None` for a proxied provider the pod cannot address the backend for
/// — no `BACKEND_API_URL`, or a checkout naming no repository — leaving the
/// upstream URL in place. That clone then fails on a credential the pod does not
/// hold, which reports the misconfiguration rather than hiding it behind a
/// request the proxy could not resolve either.
fn proxied_clone_url(repo: &RepoCheckout, backend_api_url: Option<&str>) -> Option<String> {
    if repo.provider != PROXIED_PROVIDER || repo.repository_id.is_empty() {
        return None;
    }
    let base = backend_api_url.filter(|url| !url.is_empty())?;
    Some(proxy_clone_url(base, &repo.repository_id))
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

/// How a sandbox stores its workspace so it outlives the pod.
///
/// The formats are not alternatives a reader has to choose between: a restore
/// tries the restic repository, then the mirror, then the rescue
/// `workspace.tar`, then a clone, whatever this sandbox writes.
///
/// One case does read this value. A format that holds a snapshot it could not
/// read stops the walk rather than descending to an older copy, except where the
/// format below is one this sandbox also writes and so is equally current —
/// `workspace_sync::restore_on_start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSnapshotFormat {
    /// One gzipped `workspace.tar` per sync.
    Tar,
    /// One S3 object per file plus a manifest.
    Mirror,
    /// A `restic` repository per task.
    Restic,
    /// Both, so a rollback off `restic` loses nothing. What a soak runs.
    ResticMirror,
}

impl WorkspaceSnapshotFormat {
    /// Whether a sync writes the per-file mirror.
    pub fn writes_mirror(self) -> bool {
        matches!(self, Self::Mirror | Self::ResticMirror)
    }

    /// Whether a sync writes the `restic` repository.
    pub fn writes_restic(self) -> bool {
        matches!(self, Self::Restic | Self::ResticMirror)
    }
}

/// Parse the `WORKSPACE_SNAPSHOT_FORMAT` env value.
///
/// Returns `None` for empty or unrecognised values so the caller falls back to
/// what the mirror flag says.
fn parse_snapshot_format(raw: &str) -> Option<WorkspaceSnapshotFormat> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "tar" => Some(WorkspaceSnapshotFormat::Tar),
        "mirror" => Some(WorkspaceSnapshotFormat::Mirror),
        "restic" => Some(WorkspaceSnapshotFormat::Restic),
        "restic+mirror" => Some(WorkspaceSnapshotFormat::ResticMirror),
        other => {
            tracing::warn!(
                value = %other,
                "Unrecognised WORKSPACE_SNAPSHOT_FORMAT; falling back to WORKSPACE_MIRROR_ENABLED"
            );
            None
        }
    }
}

/// Parse the `SANDBOX_PERMISSION_MODE` env value into a [`PermissionMode`].
///
/// Accepts the CLI's camelCase spellings case-insensitively (`default`,
/// `acceptEdits`, `bypassPermissions`, `plan`, `delegate`, `dontAsk`) plus a
/// couple of `snake_case` aliases. Returns `None` for empty or unrecognised
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
/// Returns `None` when the env is absent or empty. Any non-empty value is
/// preserved verbatim — including an unsupported one. Validation is not done
/// here: an unsupported harness must surface as a classified
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

/// Where uploads go when the backend supplied no `UPLOAD_DIR`.
///
/// Always inside the workspace: `workspace_sync` archives loose directories
/// alongside the repos, so an upload written here is restored with the rest of
/// the workspace instead of being re-fetched — and a path outside it would not
/// be. Kept a sibling of the repo checkouts, never inside one, so uploads stay
/// out of the user's `git status`.
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
/// Distinct from [`env_flag_enabled`], which folds "unset" into `false`. A
/// setting whose default is *on* needs that distinction: unset has to fall
/// through to the next source rather than silently turn the feature off.
fn env_flag_override(name: &str) -> Option<bool> {
    match env::var(name) {
        Ok(value) if !value.is_empty() => Some(env_flag_value_enabled(&value)),
        _ => None,
    }
}

impl Config {
    /// Absolute checkout directory of every repository placed in this sandbox,
    /// primary first. A legacy single-repo pod (no `WORKSPACE_REPOS`) falls back
    /// to [`Self::repo_dir`]. Empty entries are dropped so callers never build a
    /// path against a sandbox with nothing checked out.
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
    /// of [`Self::backend_api_url`], with any path dropped.
    ///
    /// This is the URL git matches a `credential.<url>.helper` entry against, so
    /// it must carry the scheme.
    pub fn backend_api_origin(&self) -> Option<String> {
        let (scheme, authority) = scheme_and_authority(self.backend_api_url.as_deref()?)?;
        Some(format!("{scheme}://{authority}"))
    }

    /// The `host[:port]` git sends as its `host=` field when it asks for a
    /// proxied checkout's credential.
    pub fn backend_api_host(&self) -> Option<String> {
        let (_, authority) = scheme_and_authority(self.backend_api_url.as_deref()?)?;
        Some(authority.to_string())
    }

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
        let backend_api_url = env::var("BACKEND_API_URL").ok();
        let repos = load_repo_checkouts(
            git_clone_url.as_deref(),
            git_branch.as_deref(),
            &git_provider,
            backend_api_url.as_deref(),
        );
        let repo_dir = primary_checkout(&repos)
            .map_or_else(|| workspace_dir.clone(), |repo| repo.dir(&workspace_dir));
        let upload_dir =
            env::var("UPLOAD_DIR").unwrap_or_else(|_| default_upload_dir(&workspace_dir));

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
            sandbox_id: env::var("SANDBOX_ID")
                .or_else(|_| env::var("TASK_ID"))
                .expect("SANDBOX_ID or TASK_ID is required"),

            #[allow(clippy::expect_used)] // initial member env var, crash at startup if missing
            task_id: env::var("TASK_ID").expect("TASK_ID is required"),

            agent_type: resolve_agent_type_env(),

            workspace_dir,
            repo_dir,

            data_dir: env::var("DATA_DIR")
                .unwrap_or_else(|_| "/opt/moatless/data".to_string()),

            runtime_dir: RUNTIME_DIR.to_string(),

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

            proxy: ProxyConfig {
                domain: env::var("PROXY_DOMAIN").ok(),
                protocol: env::var("PROXY_PROTOCOL")
                    .unwrap_or_else(|_| "https".to_string()),
            },
            servers: parsed.servers,
            frontend_url: env::var("FRONTEND_URL").ok(),
            default_model: env::var("DEFAULT_CLAUDE_CODE_MODEL").ok(),
            default_codex_model: env::var("DEFAULT_CODEX_MODEL").ok(),
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
            workspace_snapshot_format: env::var("WORKSPACE_SNAPSHOT_FORMAT")
                .ok()
                .and_then(|v| parse_snapshot_format(&v))
                // A backend that predates the format switch sets only the
                // mirror flag, and a sandbox has no way to ask it which it is.
                .unwrap_or_else(|| {
                    if env_flag_override("WORKSPACE_MIRROR_ENABLED").unwrap_or(true) {
                        WorkspaceSnapshotFormat::Mirror
                    } else {
                        WorkspaceSnapshotFormat::Tar
                    }
                }),
            workspace_restore_if_empty: env_flag_enabled("WORKSPACE_RESTORE_IF_EMPTY"),
            headless: env_flag_enabled("SANDBOX_HEADLESS"),

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
    /// Lives in [`RUNTIME_DIR`], the one directory that is neither archived into
    /// the workspace snapshot nor mounted into the server sidecars. Off the
    /// `workspace_dir` tree a token cannot outlive the pod it was minted for by
    /// riding the snapshot into a replacement; out of `data_dir` it is not
    /// readable by the repo code those sidecars run.
    ///
    /// How long the file survives is the runtime's business — the volume is
    /// pod-scoped under K8s and container-scoped under Docker — which is why it
    /// is one candidate in [`Self::initial_auth_token_candidates`] rather than
    /// the answer.
    pub fn persisted_user_token_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.runtime_dir).join(".user_auth_token")
    }

    /// Every token the sandbox can present on its first gRPC calls, in the order
    /// init should try them, without repeats.
    ///
    /// No single one of these is reliably the freshest. The persisted file is
    /// rewritten on every credential refresh but survives only as long as
    /// [`RUNTIME_DIR`] does; `USER_AUTH_TOKEN` and the bootstrap token are minted
    /// when the pod or container was created and then fixed. Init therefore
    /// works down this list on an `Unauthenticated` rejection instead of betting
    /// on one store.
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
    use super::{AgentConfig, Config, PermissionMode, ProxyConfig};
    use crate::config::WorkspaceSnapshotFormat;

    /// A minimal [`Config`] with empty/default values, suitable for unit tests
    /// that only exercise a few fields. Callers override what they need.
    pub(crate) fn minimal_config() -> Config {
        Config {
            port: 8001,
            grpc_port: 50052,
            sandbox_id: "test-sandbox".to_string(),
            task_id: "test-task".to_string(),
            agent_type: None,
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
            proxy: ProxyConfig::default(),
            servers: Vec::new(),
            frontend_url: None,
            default_model: None,
            default_codex_model: None,
            openai_base_url: None,
            use_tmux: false,
            command_default_timeout_secs: crate::config::DEFAULT_COMMAND_TIMEOUT_SECS,
            permission_mode: PermissionMode::BypassPermissions,
            workspace_sync_enabled: false,
            workspace_snapshot_format: WorkspaceSnapshotFormat::Mirror,
            workspace_restore_if_empty: false,
            headless: false,
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

/// Parse `REPO_CONFIG` JSON env var into server configs, agent config, and setup commands.
/// Fold the two spellings of `REPO_CONFIG`'s agent settings into one [`AgentConfig`].
///
/// `frontloadInstructions` is read from the top level, where the provisioning
/// path writes it, and falls back to the nested `agentConfig` spelling used by
/// the entrypoint-compat shape that supplies `REPO_CONFIG` verbatim. Unset at both
/// levels leaves `None`, which
/// [`AgentConfig::frontload_instructions_enabled`] reads as enabled.
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

    let agent_config =
        resolve_agent_config(repo_config.agent_config, repo_config.frontload_instructions);

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
        PermissionMode, RepoConfigFile, default_upload_dir, env_flag_override,
        env_flag_value_enabled, is_safe_mount_name, parse_permission_mode, parse_repo_checkouts,
        primary_checkout, resolve_agent_config,
    };

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

    fn agent_config_from_json(raw: &str) -> super::AgentConfig {
        let parsed: RepoConfigFile = serde_json::from_str(raw).expect("valid REPO_CONFIG");
        resolve_agent_config(parsed.agent_config, parsed.frontload_instructions)
    }

    /// A `REPO_CONFIG` that says nothing about frontloading leaves it enabled, so
    /// a repo gets its instructions in context without opting in.
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

    /// A pod provisioned before multi-repo support carries no `WORKSPACE_REPOS`;
    /// it still derives a concrete primary checkout directory from the clone URL.
    #[test]
    fn absent_workspace_repos_falls_back_to_a_derived_single_checkout() {
        let repos = parse_repo_checkouts(
            None,
            Some("https://github.com/owner/service.git"),
            Some("main"),
            "github",
            None,
        );
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].mount_name, "service");
        assert_eq!(repos[0].branch.as_deref(), Some("main"));
        assert!(repos[0].is_primary);
        assert_eq!(
            primary_checkout(&repos)
                .unwrap()
                .dir("/opt/moatless/workspace"),
            "/opt/moatless/workspace/service"
        );
    }

    #[test]
    fn no_clone_url_at_all_yields_no_checkouts() {
        assert!(parse_repo_checkouts(None, None, None, "github", None).is_empty());
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
            None,
        );
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

    /// Mount names are joined onto the workspace dir, so an entry that escapes it
    /// is dropped rather than repaired — the backend already sanitizes.
    #[test]
    fn checkouts_with_unusable_mount_names_are_dropped() {
        let raw = r#"[
            {"cloneUrl":"https://github.com/acme/api.git","provider":"github","mountName":"api","isPrimary":true},
            {"cloneUrl":"https://github.com/acme/evil.git","provider":"github","mountName":"../escape"},
            {"cloneUrl":"","provider":"github","mountName":"empty"}
        ]"#;
        let repos = parse_repo_checkouts(Some(raw), None, None, "github", None);
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
            None,
        );
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].mount_name, "repo");
    }

    /// A Forgejo token never reaches the pod, so its checkout clones from the
    /// backend and every other checkout keeps its own host.
    #[test]
    fn a_forgejo_checkout_clones_through_the_backend_proxy() {
        let raw = r#"[
            {"cloneUrl":"https://forge.example/acme/api.git","repositoryId":"repo_1","provider":"forgejo","mountName":"api","isPrimary":true},
            {"cloneUrl":"https://github.com/acme/web.git","repositoryId":"repo_2","provider":"github","mountName":"web"}
        ]"#;
        let repos = parse_repo_checkouts(
            Some(raw),
            None,
            None,
            "forgejo",
            Some("http://backend:8001/"),
        );
        assert_eq!(repos[0].clone_url, "http://backend:8001/git/repo_1");
        assert_eq!(repos[1].clone_url, "https://github.com/acme/web.git");
    }

    /// With nothing to address the backend by, the upstream URL stays: the clone
    /// then fails on the credential the pod does not hold, which is a report.
    #[test]
    fn a_forgejo_checkout_the_proxy_cannot_be_addressed_for_keeps_its_remote() {
        let raw = r#"[{"cloneUrl":"https://forge.example/acme/api.git","repositoryId":"repo_1","provider":"forgejo","mountName":"api","isPrimary":true}]"#;
        assert_eq!(
            parse_repo_checkouts(Some(raw), None, None, "forgejo", None)[0].clone_url,
            "https://forge.example/acme/api.git"
        );

        let no_id = r#"[{"cloneUrl":"https://forge.example/acme/api.git","provider":"forgejo","mountName":"api","isPrimary":true}]"#;
        assert_eq!(
            parse_repo_checkouts(Some(no_id), None, None, "forgejo", Some("http://backend:8001"))[0]
                .clone_url,
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
        assert_eq!(config.backend_api_host().as_deref(), Some("backend.svc:8001"));

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
        let raw = r#"[{"cloneUrl":"https://github.com/acme/api.git","provider":"github","mountName":"api"}]"#;
        let repos = parse_repo_checkouts(Some(raw), None, None, "github", None);
        assert_eq!(primary_checkout(&repos).unwrap().mount_name, "api");
        assert!(primary_checkout(&[]).is_none());
    }
}
