//! Build agent prompt/options from sandbox config + per-request overrides.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use claude_agent_sdk::types::options::{PluginConfig, SystemPrompt};
use claude_agent_sdk::{ClaudeOptions, PermissionMode};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agent::handle::{resolve_agent_harness, AGENT_TYPE_CODEX};
use crate::agent::types::AgentRunRequest;
use crate::config::Config;
use crate::events::GrpcEventPublisher;

use super::agent_state::AgentState;
use super::permission_handler::InteractivePermissionHandler;

/// Build ClaudeOptions from config + per-request overrides.
pub fn build_options(
    config: &Config,
    request: &AgentRunRequest,
    cancel_token: CancellationToken,
    agent_state: Arc<AgentState>,
    env: Option<HashMap<String, String>>,
    events: Option<GrpcEventPublisher>,
) -> ClaudeOptions {
    // Determine model: request > agentConfig > DEFAULT_CLAUDE_CODE_MODEL env
    let model = request
        .model
        .clone()
        .or_else(|| config.agent_config.model.clone())
        .or_else(|| config.default_model.clone());

    // Build system prompt
    let system_prompt = build_system_prompt(config, request);

    // Build plugin configs from paths
    let plugins = build_plugins(config);

    // Detect plan mode from agent_mode.
    let is_plan_mode = request.agent_mode.as_deref() == Some("plan");

    // Build the permission handler for interactive prompts (AskUserQuestion,
    // ExitPlanMode). It also enforces plan-mode write blocking when active, and
    // publishes each prompt it parks on so the question reaches a person.
    let permission_handler =
        InteractivePermissionHandler::with_plan_mode(agent_state, is_plan_mode);
    let permission_handler = match events {
        Some(events) => permission_handler.announcing_prompts(events, config.task_id.clone()),
        None => permission_handler,
    };

    // Determine session resume
    let (resume, continue_conversation) = resolve_resume(request);
    let init_timeout = std::time::Duration::from_mins(2);

    // tmux mode always launches the TUI with --dangerously-skip-permissions,
    // which the CLI refuses under root unless IS_SANDBOX=1. The cluster sets
    // it via gitops extra env; Docker mode doesn't — set it for the child
    // unconditionally so the transport works in both runtimes.
    let env = if config.use_tmux {
        let mut env = env.unwrap_or_default();
        env.entry("IS_SANDBOX".to_string())
            .or_insert_with(|| "1".to_string());
        Some(env)
    } else {
        // Subprocess mode: each turn is a one-shot `claude -p` process. Claude
        // Code terminates any CLI-managed `run_in_background` Bash shell a few
        // seconds after the final result once stdin closes (see
        // https://code.claude.com/docs/en/headless), so a job the agent
        // "backgrounds" this way is killed at turn end rather than left
        // running — it never surfaces as a live process and never completes.
        // Disable that path so the agent keeps work in the foreground instead
        // of starting jobs that die silently. A genuinely detached process
        // (`setsid`/`nohup … &`) is not CLI-managed, survives the turn, and is
        // still covered by the post-turn background-completion watch.
        let mut env = env.unwrap_or_default();
        env.entry("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS".to_string())
            .or_insert_with(|| "1".to_string());
        Some(env)
    };

    info!(
        task_id = %config.task_id,
        model = ?model,
        resume = ?resume,
        continue_conversation = ?continue_conversation,
        cli_path = %config.claude_cli_path,
        init_timeout_secs = init_timeout.as_secs(),
        is_plan_mode,
        "Built agent options"
    );

    ClaudeOptions {
        model,
        effort: claude_effort(request.effort.as_deref()),
        system_prompt,
        // Plan-mode requests always use Plan so the InteractivePermissionHandler
        // can block writes until ExitPlanMode. Otherwise use the configured mode
        // (`SANDBOX_PERMISSION_MODE`), which defaults to BypassPermissions:
        // AcceptEdits auto-approves only file edit/write tools and leaves network
        // tools like WebSearch/WebFetch denied, since the CLI never routes those
        // through the stdio permission handler.
        permission_mode: Some(if is_plan_mode {
            PermissionMode::Plan
        } else {
            config.permission_mode.clone()
        }),
        cwd: Some(config.workspace_dir.clone()),
        cli_path: Some(config.claude_cli_path.clone()),
        plugins: if plugins.is_empty() {
            None
        } else {
            Some(plugins)
        },
        resume,
        continue_conversation,
        can_use_tool: Some(Arc::new(permission_handler)),
        cancellation_token: Some(cancel_token),
        max_turns: config.agent_config.timeout.map(|t| t as u32),
        init_timeout: Some(init_timeout),
        env,
        // Disallowed tools are removed from the CLI's context entirely (a bare
        // tool name in the deny list), so the agent never sees or calls them.
        //
        // - Monitor (subprocess mode only): the CLI's built-in Monitor tool
        //   only delivers events while its own process is alive. The
        //   subprocess per-turn `--resume` model kills that process at turn
        //   end, so the Monitor watcher dies before any events reach the
        //   agent. Disallow it so the agent gets a loud "tool not allowed"
        //   error and falls back to Bash background. In tmux mode the TUI
        //   process persists across turns, so Monitor (and the CLI's internal
        //   ScheduleWakeup timer) work natively and Monitor stays allowed.
        //
        // - The interactive tools (headless tasks only): AskUserQuestion and
        //   Enter/ExitPlanMode each park the turn until a person answers. A
        //   headless task has nobody watching for the prompt, so the turn would
        //   sit on a question that is never answered. Where a person is
        //   watching, the question reaches them as a durable activity and their
        //   answer comes back through the permission handler, so they stay
        //   allowed and the prompt is rendered rather than dropped.
        disallowed_tools: Some(disallowed_tools(config, is_plan_mode)),
        ..ClaudeOptions::default()
    }
}

/// The tools kept out of the CLI's context for this run.
///
/// See the call site in [`build_options`] for why each one is on the list. A
/// plan-mode run keeps `ExitPlanMode`: it is the only way out of plan mode, and
/// asking for plan mode is asking for the approval prompt it raises.
fn disallowed_tools(config: &Config, is_plan_mode: bool) -> Vec<String> {
    let mut tools: Vec<&str> = Vec::new();
    if !config.use_tmux {
        tools.push("Monitor");
    }
    if config.headless {
        tools.push("AskUserQuestion");
        tools.push("EnterPlanMode");
        if !is_plan_mode {
            tools.push("ExitPlanMode");
        }
    }
    tools.into_iter().map(String::from).collect()
}

/// Build the system prompt configuration.
#[allow(clippy::unnecessary_wraps)] // Option for caller API consistency
fn build_system_prompt(config: &Config, request: &AgentRunRequest) -> Option<SystemPrompt> {
    Some(SystemPrompt::Preset {
        prompt_type: "preset".to_string(),
        preset: "claude_code".to_string(),
        append: Some(build_prompt_append(config, request)),
    })
}

/// The effort levels the Claude CLI's `--effort` accepts. Anything else it
/// warns about and ignores, so this module filters instead of forwarding.
const CLAUDE_EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// The `--effort` value for a requested effort, or `None` to leave the CLI on
/// its own default.
///
/// `ultracode` is a Claude Code setting rather than a CLI level and stands for
/// the highest reasoning the model offers, so it resolves to `xhigh`. A level
/// this CLI does not know (`ultrathink`, which is prompt-injected where it is
/// implemented at all) is dropped with a warning rather than passed through for
/// the CLI to ignore silently.
fn claude_effort(requested: Option<&str>) -> Option<String> {
    let requested = requested.map(str::trim).filter(|value| !value.is_empty())?;
    let level = match requested.to_ascii_lowercase().as_str() {
        "ultracode" => "xhigh".to_string(),
        other if CLAUDE_EFFORT_LEVELS.contains(&other) => other.to_string(),
        other => {
            warn!(effort = %other, "Unsupported Claude effort level; using the CLI default");
            return None;
        }
    };
    Some(level)
}

/// Return the backend-supplied append-system-prompt, prefixed with a note
/// describing the workspace layout.
///
/// The backend assembles the bulk of the prompt (task identity, repo/channel
/// system prompts, preview servers, task URL, CLI reference, GitHub hint) when
/// the coordinator fetches the system prompt. On top of that, the sandbox
/// prepends a short explanation of the on-disk layout, since only the
/// sandbox knows the concrete `workspace_dir`/`repo_dir` paths for this pod:
/// the workspace dir is the persisted working area (the agent's cwd, synced to
/// durable storage), and the repo dir is the checkout of the task repository
/// inside it.
pub fn build_prompt_append(config: &Config, request: &AgentRunRequest) -> String {
    let backend_prompt = request.system_prompt.clone().unwrap_or_default();
    let layout_note = build_workspace_layout_note(config);
    let mut sections = vec![layout_note];
    if let Some(repo_instructions) = build_repo_instructions_section(config) {
        sections.push(repo_instructions);
    }
    if !backend_prompt.is_empty() {
        sections.push(backend_prompt);
    }
    let append = sections.join("\n\n");
    info!(
        append_system_prompt_len = append.len(),
        has_request_prompt = request.system_prompt.is_some(),
        "Assembled system prompt"
    );
    append
}

/// Build the workspace-layout note prepended to every system prompt.
///
/// Explains the split between the workspace dir (the agent's cwd) and the repo
/// dir (the checkout of the task's repository). The note is deliberately precise
/// about *what persists*: the task repository is snapshotted to durable storage
/// (its committed history plus tracked/untracked-non-ignored working-tree
/// changes), so repo work survives sandbox restarts. Files written outside the
/// repository — loose files in the workspace dir — are currently ephemeral. A
/// workspace that places several repositories lists the others too, so the agent
/// knows they were checked out for it rather than assuming a stray clone. When
/// there is no separate repo checkout (`repo_dir == workspace_dir`) the note
/// collapses to a single paragraph.
fn build_workspace_layout_note(config: &Config) -> String {
    let workspace_dir = &config.workspace_dir;
    if config.repo_dir == config.workspace_dir {
        format!(
            "## Workspace layout\n\
             Your working directory is `{workspace_dir}`, which holds the task's git \
             repository. Committed history plus your uncommitted, non-gitignored \
             working-tree changes are snapshotted to durable storage and survive \
             sandbox restarts. Build artifacts and other gitignored paths are not \
             snapshotted (they are regenerated by setup commands)."
        )
    } else {
        let repo_dir = &config.repo_dir;
        let secondary: Vec<String> = config
            .repos
            .iter()
            .map(|repo| repo.dir(workspace_dir))
            .filter(|dir| dir != repo_dir)
            .map(|dir| format!("`{dir}`"))
            .collect();
        let (others, persisted) = if secondary.is_empty() {
            (String::new(), "Only the repository is persisted")
        } else {
            (
                format!(
                    "- Also checked out beside it, as part of this workspace: {}. Work in \
                     them the same way; they are part of the task, not stray clones.\n",
                    secondary.join(", ")
                ),
                "Only the repositories are persisted",
            )
        };
        format!(
            "## Workspace layout\n\
             - Workspace dir: `{workspace_dir}` — your working directory (cwd). The \
             task repository is checked out in a subdirectory of it (see below).\n\
             - Repo dir: `{repo_dir}` — the checkout of the task's repository; do your \
             repository work here. Its committed history plus your uncommitted, \
             non-gitignored working-tree changes are snapshotted to durable storage \
             and survive sandbox restarts.\n\
             {others}\
             {persisted} right now: loose files elsewhere under \
             the workspace dir (and gitignored build artifacts) are ephemeral and may \
             be lost when the sandbox is recycled."
        )
    }
}

/// The root instruction filenames to look for, in preference order, for the
/// harness this sandbox runs: Claude reads `CLAUDE.md` and Codex reads
/// `AGENTS.md`, so the agent is pointed at the file it would have loaded itself.
/// A repo commits one of the two (often as a symlink to the other), so the other
/// name is the fallback.
fn repo_instruction_filenames(config: &Config) -> [&'static str; 2] {
    let is_codex = config
        .agent_type
        .as_deref()
        .and_then(|raw| resolve_agent_harness(raw).ok())
        == Some(AGENT_TYPE_CODEX);
    if is_codex {
        ["AGENTS.md", "CLAUDE.md"]
    } else {
        ["CLAUDE.md", "AGENTS.md"]
    }
}

/// Build the "# Repository Instructions" section: the path of each checked-out
/// repo's root instruction file, for the agent to read before working in that
/// repo. The contents are deliberately not included — a multi-repo workspace
/// would spend most of the prompt on instructions for repos the task never
/// touches.
///
/// These files are kept out of the agents' on-the-fly memory loading (Claude's
/// `claudeMdExcludes`, Codex's `project_doc_max_bytes = 0`; both set in `init`),
/// so this listing is what tells the agent they exist.
///
/// Returns `None` when no checked-out repo has a root instruction file.
fn build_repo_instructions_section(config: &Config) -> Option<String> {
    let filenames = repo_instruction_filenames(config);
    let listed: Vec<String> = config
        .repo_dirs()
        .iter()
        .filter_map(|dir| repo_instruction_path(dir, filenames))
        .map(|path| format!("- `{path}`"))
        .collect();

    if listed.is_empty() {
        return None;
    }

    Some(format!(
        "# Repository Instructions\n\n\
         Each repository cloned into this workspace carries its own instruction file. \
         Read the one for a repository before doing anything in it:\n{}",
        listed.join("\n")
    ))
}

/// The path of a repo's root instruction file: the first of `filenames` that
/// exists. Existence follows symlinks (the `CLAUDE.md -> AGENTS.md` convention),
/// so either committed name resolves.
fn repo_instruction_path(dir: &str, filenames: [&'static str; 2]) -> Option<String> {
    filenames.into_iter().find_map(|filename| {
        let path = Path::new(dir).join(filename);
        path.exists().then(|| path.to_string_lossy().into_owned())
    })
}

/// Build plugin configs from the plugins cloned into the sandbox.
fn build_plugins(_config: &Config) -> Vec<PluginConfig> {
    build_plugins_with_root(Path::new("/opt/moatless/plugins"))
}

fn build_plugins_with_root(plugins_dir: &Path) -> Vec<PluginConfig> {
    let mut plugins: Vec<PluginConfig> = Vec::new();

    if plugins_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    for plugin_path in plugin_dirs_for_repo(&path) {
                        plugins.push(PluginConfig {
                            plugin_type: "local".to_string(),
                            path: plugin_path.to_string_lossy().to_string(),
                        });
                    }
                }
            }
        }
    }

    plugins
}

#[derive(Debug, Deserialize)]
struct PluginMarketplace {
    #[serde(default)]
    plugins: Vec<PluginMarketplaceEntry>,
}

#[derive(Debug, Deserialize)]
struct PluginMarketplaceEntry {
    name: String,
    source: PluginMarketplaceSource,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PluginMarketplaceSource {
    LocalPath(String),
    Descriptor {
        source: String,
        #[serde(default)]
        path: Option<String>,
    },
}

const PLUGIN_MARKETPLACE_PATHS: [&str; 2] = [
    ".agents/plugins/marketplace.json",
    ".claude-plugin/marketplace.json",
];

fn plugin_dirs_for_repo(repo_path: &Path) -> Vec<PathBuf> {
    let Some((marketplace_path, raw)) = read_first_marketplace(repo_path) else {
        return vec![repo_path.to_path_buf()];
    };

    let marketplace: PluginMarketplace = match serde_json::from_str(&raw) {
        Ok(marketplace) => marketplace,
        Err(e) => {
            warn!(
                marketplace = %marketplace_path.display(),
                error = %e,
                "Invalid plugin marketplace — falling back to cloned repo root"
            );
            return vec![repo_path.to_path_buf()];
        }
    };

    marketplace
        .plugins
        .into_iter()
        .filter_map(|plugin| {
            let source_path = marketplace_local_path(&plugin.source, &plugin.name)?;
            let plugin_subdir = match normalize_marketplace_path(&source_path) {
                Ok(path) => path,
                Err(e) => {
                    warn!(
                        plugin = %plugin.name,
                        path = %source_path,
                        error = %e,
                        "Rejected plugin marketplace source path"
                    );
                    return None;
                }
            };
            let plugin_path = repo_path.join(plugin_subdir);
            if plugin_path.is_dir() {
                Some(plugin_path)
            } else {
                warn!(
                    plugin = %plugin.name,
                    path = %plugin_path.display(),
                    "Plugin marketplace local source directory is missing"
                );
                None
            }
        })
        .collect()
}

fn read_first_marketplace(repo_path: &Path) -> Option<(PathBuf, String)> {
    for marketplace_path in PLUGIN_MARKETPLACE_PATHS {
        let path = repo_path.join(marketplace_path);
        if let Ok(raw) = std::fs::read_to_string(&path) {
            return Some((path, raw));
        }
    }
    None
}

fn marketplace_local_path(source: &PluginMarketplaceSource, plugin_name: &str) -> Option<String> {
    match source {
        PluginMarketplaceSource::LocalPath(path) => {
            if path.trim().starts_with("./") {
                Some(path.clone())
            } else {
                debug!(
                    plugin = %plugin_name,
                    source = %path,
                    "Skipping non-local plugin marketplace string source"
                );
                None
            }
        }
        PluginMarketplaceSource::Descriptor { source, path } if source == "local" => path.clone(),
        PluginMarketplaceSource::Descriptor { source, .. } => {
            debug!(
                plugin = %plugin_name,
                source = %source,
                "Skipping non-local plugin marketplace descriptor source"
            );
            None
        }
    }
}

fn normalize_marketplace_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('/') {
        return Err("plugin marketplace source path must be repo-relative".to_string());
    }
    let stripped = trimmed
        .strip_prefix("./")
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    if stripped.split('/').any(|segment| segment == "..") {
        return Err("plugin marketplace source path must not contain '..' segments".to_string());
    }
    Ok(stripped.to_string())
}

/// Resolve session resume from request.
///
/// Returns `(resume_session_id, continue_conversation)`:
/// - `WithId(id)` → `--resume <id>`
/// - `Last` → `--continue` (let the CLI find the latest session)
fn resolve_resume(request: &AgentRunRequest) -> (Option<String>, Option<bool>) {
    use super::types::ResumeSession;
    match &request.resume {
        Some(ResumeSession::WithId(session_id)) => (Some(session_id.clone()), None),
        Some(ResumeSession::Last) => (None, Some(true)),
        None => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::{AgentConfig, ProxyConfig};
    use moatless_api_schemas::sandbox::RepoCheckout;
    use std::fs;
    use tempfile::tempdir;

    fn test_config(use_tmux: bool) -> Config {
        Config {
            port: 0,
            grpc_port: 0,
            task_id: "task-test".to_string(),
            agent_type: None,
            workspace_dir: "/tmp/workspace".to_string(),
            repo_dir: "/tmp/workspace".to_string(),
            data_dir: "/tmp/data".to_string(),
            upload_dir: "/tmp/uploads".to_string(),
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
            default_model: None,
            default_codex_model: None,
            use_tmux,
            background_job_monitoring_enabled: false,
            permission_mode: claude_agent_sdk::PermissionMode::BypassPermissions,
            workspace_sync_enabled: false,
            headless: false,
            home_dir: "/tmp/home".to_string(),
            codex_home_dir: "/tmp/home/.codex".to_string(),
            claude_config_dir: "/tmp/home/.claude".to_string(),
            git_clone_url: None,
            git_branch: None,
            git_provider: "github".to_string(),
            repos: Vec::new(),
            repo_config_raw: None,
            setup_commands: vec![],
            plugins: vec![],
        }
    }

    fn test_request() -> AgentRunRequest {
        AgentRunRequest {
            content: Vec::new(),
            model: None,
            system_prompt: None,
            resume: None,
            agent_type: None,
            agent_mode: None,
            effort: None,
        }
    }

    fn build_test_options(use_tmux: bool) -> ClaudeOptions {
        build_options(
            &test_config(use_tmux),
            &test_request(),
            CancellationToken::new(),
            Arc::new(AgentState::new()),
            None,
            None,
        )
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn build_plugins_finds_cloned_plugin_dir() {
        let root = tempdir().unwrap();
        let plugins_root = root.path().join("plugins");
        let direct = plugins_root.join("direct-plugin");
        write(
            &direct.join(".claude-plugin/plugin.json"),
            r#"{"name":"direct-plugin"}"#,
        );

        let plugins = build_plugins_with_root(&plugins_root);

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].plugin_type, "local");
        assert_eq!(plugins[0].path, direct.to_string_lossy());
    }

    #[test]
    fn build_plugins_expands_claude_marketplace_local_sources() {
        let root = tempdir().unwrap();
        let plugins_root = root.path().join("plugins");
        let repo = plugins_root.join("moatless");
        write(
            &repo.join(".claude-plugin/marketplace.json"),
            r#"{
                "name": "moatless-skills",
                "plugins": [
                    {
                        "name": "moatless-skills",
                        "source": "./plugins/moatless-skills"
                    }
                ]
            }"#,
        );
        let nested = repo.join("plugins/moatless-skills");
        write(
            &nested.join(".claude-plugin/plugin.json"),
            r#"{"name":"moatless-skills"}"#,
        );

        let plugins = build_plugins_with_root(&plugins_root);

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].plugin_type, "local");
        assert_eq!(plugins[0].path, nested.to_string_lossy());
    }

    #[test]
    fn build_plugins_expands_agents_marketplace_local_sources() {
        let root = tempdir().unwrap();
        let plugins_root = root.path().join("plugins");
        let repo = plugins_root.join("moatless");
        write(
            &repo.join(".agents/plugins/marketplace.json"),
            r#"{
                "name": "moatless",
                "plugins": [
                    {
                        "name": "moatless-skills",
                        "source": {
                            "source": "local",
                            "path": "./plugins/moatless-skills"
                        }
                    }
                ]
            }"#,
        );
        let nested = repo.join("plugins/moatless-skills");
        write(
            &nested.join(".codex-plugin/plugin.json"),
            r#"{"name":"moatless-skills"}"#,
        );

        let plugins = build_plugins_with_root(&plugins_root);

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].plugin_type, "local");
        assert_eq!(plugins[0].path, nested.to_string_lossy());
    }

    #[test]
    fn build_plugins_skips_nonlocal_marketplace_sources() {
        let root = tempdir().unwrap();
        let plugins_root = root.path().join("plugins");
        let repo = plugins_root.join("marketplace");
        write(
            &repo.join(".claude-plugin/marketplace.json"),
            r#"{
                "name": "remote",
                "plugins": [
                    {
                        "name": "remote-plugin",
                        "source": "https://github.com/example/remote-plugin"
                    }
                ]
            }"#,
        );

        let plugins = build_plugins_with_root(&plugins_root);

        assert!(plugins.is_empty());
    }

    #[test]
    fn normal_mode_uses_configured_permission_mode() {
        // Default config carries BypassPermissions, which non-plan runs adopt.
        let options = build_test_options(false);
        assert_eq!(
            options.permission_mode,
            Some(PermissionMode::BypassPermissions)
        );

        // An overridden config mode flows through for non-plan runs.
        let mut config = test_config(false);
        config.permission_mode = PermissionMode::AcceptEdits;
        let options = build_options(
            &config,
            &test_request(),
            CancellationToken::new(),
            Arc::new(AgentState::new()),
            None,
            None,
        );
        assert_eq!(options.permission_mode, Some(PermissionMode::AcceptEdits));
    }

    #[test]
    fn plan_mode_overrides_configured_permission_mode() {
        let mut config = test_config(false);
        config.permission_mode = PermissionMode::BypassPermissions;
        let mut request = test_request();
        request.agent_mode = Some("plan".to_string());
        let options = build_options(
            &config,
            &request,
            CancellationToken::new(),
            Arc::new(AgentState::new()),
            None,
            None,
        );
        assert_eq!(options.permission_mode, Some(PermissionMode::Plan));
    }

    #[test]
    fn a_requested_effort_reaches_the_cli() {
        let mut request = test_request();
        request.effort = Some("max".to_string());
        let options = build_options(
            &test_config(false),
            &request,
            CancellationToken::new(),
            Arc::new(AgentState::new()),
            None,
            None,
        );
        assert_eq!(options.effort.as_deref(), Some("max"));
    }

    #[test]
    fn no_requested_effort_leaves_the_cli_on_its_default() {
        assert_eq!(build_test_options(false).effort, None);
    }

    #[test]
    fn claude_effort_accepts_every_level_the_cli_does() {
        for level in CLAUDE_EFFORT_LEVELS {
            assert_eq!(claude_effort(Some(level)).as_deref(), Some(*level));
        }
    }

    #[test]
    fn claude_effort_is_case_insensitive_and_trims() {
        assert_eq!(claude_effort(Some("  XHigh ")).as_deref(), Some("xhigh"));
    }

    /// `ultracode` names the highest reasoning level rather than a CLI one, so
    /// a selection carried over from a client that offers it still runs high.
    #[test]
    fn claude_effort_maps_ultracode_to_the_highest_cli_level() {
        assert_eq!(claude_effort(Some("ultracode")).as_deref(), Some("xhigh"));
    }

    /// The CLI warns-and-ignores an unknown level, so forwarding one would look
    /// like it took effect. Dropping it says the same thing out loud.
    #[test]
    fn claude_effort_drops_a_level_the_cli_does_not_know() {
        assert_eq!(claude_effort(Some("ultrathink")), None);
        assert_eq!(claude_effort(Some("bogus")), None);
        assert_eq!(claude_effort(Some("   ")), None);
        assert_eq!(claude_effort(None), None);
    }

    #[test]
    fn disallowed_tools_subprocess_mode_blocks_monitor() {
        let options = build_test_options(false);
        let disallowed = options.disallowed_tools.expect("disallowed_tools set");
        assert!(disallowed.contains(&"Monitor".to_string()));
        assert!(!disallowed.contains(&"AskUserQuestion".to_string()));
    }

    #[test]
    fn disallowed_tools_tmux_mode_allows_monitor() {
        let options = build_test_options(true);
        let disallowed = options.disallowed_tools.expect("disallowed_tools set");
        assert!(!disallowed.contains(&"Monitor".to_string()));
        assert!(!disallowed.contains(&"AskUserQuestion".to_string()));
    }

    /// The tools that block a turn on a person's answer.
    const INTERACTIVE_TOOLS: &[&str] = &["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];

    /// A headless task has nobody to answer a prompt, so the tools that wait on
    /// one are kept out of the agent's context in both transports.
    #[test]
    fn headless_blocks_the_interactive_tools() {
        for use_tmux in [false, true] {
            let mut config = test_config(use_tmux);
            config.headless = true;
            let disallowed = disallowed_tools(&config, false);
            for tool in INTERACTIVE_TOOLS {
                assert!(
                    disallowed.contains(&(*tool).to_string()),
                    "{tool} must be disallowed when headless (use_tmux={use_tmux})"
                );
            }
        }
    }

    /// A task someone is watching keeps them: the prompt reaches a person and
    /// the answer comes back through the permission handler.
    #[test]
    fn a_watched_task_keeps_the_interactive_tools() {
        for use_tmux in [false, true] {
            let disallowed = disallowed_tools(&test_config(use_tmux), false);
            for tool in INTERACTIVE_TOOLS {
                assert!(
                    !disallowed.contains(&(*tool).to_string()),
                    "{tool} must stay allowed when not headless (use_tmux={use_tmux})"
                );
            }
        }
    }

    /// Plan mode blocks every write until `ExitPlanMode`, so a run that asked
    /// for it keeps the one tool that ends it — even headless.
    #[test]
    fn a_headless_plan_run_keeps_exit_plan_mode() {
        let mut config = test_config(false);
        config.headless = true;
        let disallowed = disallowed_tools(&config, true);
        assert!(!disallowed.contains(&"ExitPlanMode".to_string()));
        assert!(disallowed.contains(&"AskUserQuestion".to_string()));
        assert!(disallowed.contains(&"EnterPlanMode".to_string()));
    }

    #[test]
    fn tmux_mode_sets_is_sandbox_env() {
        // The TUI refuses --dangerously-skip-permissions under root unless
        // IS_SANDBOX=1 — tmux mode must always set it for the child.
        let options = build_test_options(true);
        let env = options.env.expect("env set in tmux mode");
        assert_eq!(env.get("IS_SANDBOX").map(String::as_str), Some("1"));
        // tmux keeps the persistent session alive, so CLI-managed background
        // tasks work there and must NOT be disabled.
        assert!(!env.contains_key("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS"));
    }

    #[test]
    fn subprocess_mode_disables_cli_background_tasks() {
        // CLI-managed `run_in_background` Bash shells are killed shortly after
        // the one-shot `claude -p` turn ends, so subprocess mode disables them.
        let options = build_test_options(false);
        let env = options.env.expect("env set in subprocess mode");
        assert_eq!(
            env.get("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS")
                .map(String::as_str),
            Some("1")
        );
        // IS_SANDBOX is a tmux-only flag; it must not leak into subprocess mode.
        assert!(!env.contains_key("IS_SANDBOX"));
    }

    #[test]
    fn layout_note_single_paragraph_when_repo_equals_workspace() {
        let config = test_config(false); // repo_dir == workspace_dir
        let note = build_workspace_layout_note(&config);
        assert!(note.contains("## Workspace layout"));
        assert!(note.contains("/tmp/workspace"));
        // No two-bullet split when there's no separate repo checkout.
        assert!(!note.contains("- Repo dir:"));
    }

    #[test]
    fn layout_note_splits_workspace_and_repo_dirs() {
        let mut config = test_config(false);
        config.workspace_dir = "/opt/moatless/workspace".to_string();
        config.repo_dir = "/opt/moatless/workspace/my-repo".to_string();
        let note = build_workspace_layout_note(&config);
        assert!(note.contains("Workspace dir: `/opt/moatless/workspace`"));
        assert!(note.contains("Repo dir: `/opt/moatless/workspace/my-repo`"));
        // Phase 1 honesty: only the repo persists, loose workspace files are ephemeral.
        assert!(note.contains("Only the repository is persisted"));
        // A single-repo workspace has nothing else to list.
        assert!(!note.contains("Also checked out beside it"));
    }

    /// A multi-repo workspace must tell the agent the sibling checkouts are part
    /// of the task, not stray clones it should ignore.
    #[test]
    fn layout_note_lists_the_workspaces_other_repos() {
        let mut config = test_config(false);
        config.workspace_dir = "/opt/moatless/workspace".to_string();
        config.repo_dir = "/opt/moatless/workspace/api".to_string();
        config.repos = vec![
            RepoCheckout {
                clone_url: "https://github.com/acme/api.git".into(),
                branch: None,
                provider: "github".into(),
                mount_name: "api".into(),
                is_primary: true,
            },
            RepoCheckout {
                clone_url: "https://github.com/acme/web.git".into(),
                branch: None,
                provider: "github".into(),
                mount_name: "web".into(),
                is_primary: false,
            },
        ];
        let note = build_workspace_layout_note(&config);
        assert!(note.contains("Repo dir: `/opt/moatless/workspace/api`"));
        assert!(note.contains("`/opt/moatless/workspace/web`"));
        assert!(note.contains("Also checked out beside it"));
        assert!(note.contains("Only the repositories are persisted"));
    }

    #[test]
    fn prompt_append_prepends_layout_note_before_backend_prompt() {
        let config = test_config(false);
        let mut request = test_request();
        request.system_prompt = Some("BACKEND-PROMPT".to_string());
        let append = build_prompt_append(&config, &request);
        let note_idx = append.find("## Workspace layout").expect("note present");
        let backend_idx = append
            .find("BACKEND-PROMPT")
            .expect("backend prompt present");
        assert!(note_idx < backend_idx, "layout note must come first");
    }

    fn repo(mount_name: &str, is_primary: bool) -> RepoCheckout {
        RepoCheckout {
            clone_url: "https://example.com/repo.git".into(),
            branch: None,
            provider: "github".into(),
            mount_name: mount_name.into(),
            is_primary,
        }
    }

    /// Each repo's root instruction file is listed by full path, never inlined;
    /// a repo carrying only the other filename is picked up via the fallback.
    #[test]
    fn repo_instructions_section_lists_each_repos_root_file_by_path() {
        let root = tempdir().unwrap();
        let ws = root.path().to_string_lossy().to_string();
        let api_dir = format!("{ws}/api");
        let web_dir = format!("{ws}/web");
        write(&PathBuf::from(&api_dir).join("CLAUDE.md"), "API RULES");
        write(&PathBuf::from(&web_dir).join("AGENTS.md"), "WEB RULES");

        let mut config = test_config(false);
        config.workspace_dir = ws;
        config.repo_dir = api_dir.clone();
        config.repos = vec![repo("api", true), repo("web", false)];

        let mut request = test_request();
        request.system_prompt = Some("BACKEND-PROMPT".to_string());
        let append = build_prompt_append(&config, &request);

        assert!(append.contains("# Repository Instructions"));
        assert!(append.contains("Read the one for a repository before doing anything in it:"));
        assert!(append.contains(&format!("- `{api_dir}/CLAUDE.md`")));
        // Only AGENTS.md exists here, so the fallback name is what gets listed.
        assert!(append.contains(&format!("- `{web_dir}/AGENTS.md`")));
        // The files are pointed at, never inlined.
        assert!(!append.contains("API RULES"));
        assert!(!append.contains("WEB RULES"));

        // Order: layout note, then repo instructions, then the backend prompt.
        let layout_idx = append.find("## Workspace layout").unwrap();
        let repo_idx = append.find("# Repository Instructions").unwrap();
        let backend_idx = append.find("BACKEND-PROMPT").unwrap();
        assert!(layout_idx < repo_idx && repo_idx < backend_idx);
    }

    /// A repo committing both names is pointed at under the one the running agent
    /// would have loaded itself: `CLAUDE.md` for Claude, `AGENTS.md` for Codex.
    #[test]
    fn repo_instructions_filename_follows_the_agent() {
        let root = tempdir().unwrap();
        let ws = root.path().to_string_lossy().to_string();
        let dir = format!("{ws}/api");
        write(&PathBuf::from(&dir).join("CLAUDE.md"), "CLAUDE COPY");
        write(&PathBuf::from(&dir).join("AGENTS.md"), "AGENTS COPY");

        let mut config = test_config(false);
        config.workspace_dir = ws;
        config.repo_dir = dir.clone();
        config.repos = vec![repo("api", true)];

        config.agent_type = Some("claude-code".to_string());
        let claude = build_prompt_append(&config, &test_request());
        assert!(claude.contains(&format!("- `{dir}/CLAUDE.md`")));
        assert!(!claude.contains(&format!("- `{dir}/AGENTS.md`")));

        config.agent_type = Some("codex".to_string());
        let codex = build_prompt_append(&config, &test_request());
        assert!(codex.contains(&format!("- `{dir}/AGENTS.md`")));
        assert!(!codex.contains(&format!("- `{dir}/CLAUDE.md`")));
    }

    /// Claude's transport/model variants are the same harness, and an unset or
    /// unknown type is not Codex — all keep the `CLAUDE.md` preference.
    #[test]
    fn repo_instruction_filenames_default_to_claude() {
        let mut config = test_config(false);
        for agent_type in [
            None,
            Some("claude-code-tui"),
            Some("claude-code-fable"),
            Some("gpt-9000"),
        ] {
            config.agent_type = agent_type.map(str::to_string);
            assert_eq!(
                repo_instruction_filenames(&config),
                ["CLAUDE.md", "AGENTS.md"],
                "agent_type={agent_type:?}"
            );
        }
    }

    /// A repo with no root instruction file adds no section rather than an empty
    /// heading.
    #[test]
    fn repo_instructions_section_absent_without_instruction_files() {
        let root = tempdir().unwrap();
        let ws = root.path().to_string_lossy().to_string();
        let mut config = test_config(false);
        config.repo_dir = format!("{ws}/api");
        config.workspace_dir = ws;
        config.repos = vec![repo("api", true)];

        let append = build_prompt_append(&config, &test_request());
        assert!(!append.contains("# Repository Instructions"));
    }
}
