//! Build agent prompt/options from sandbox config + per-request overrides.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use claude_agent_sdk::api::options::{PluginConfig, SystemPrompt};
use claude_agent_sdk::{ClaudeOptions, PermissionMode};
use moatless_types::model::{CLAUDE_EFFORT_LEVELS, ClaudeModel, claude_model_id};
use moatless_workspace_types::plugin::{
    PLUGIN_MARKETPLACE_PATHS, PluginMarketplaceSource, marketplace_local_path,
};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::agent::handle::resolve_agent_harness;
use crate::agent::types::AgentRunRequest;
use crate::config::Config;

use super::agent_state::AgentState;
use super::permission_handler::InteractivePermissionHandler;

/// Claude CLI environment defaults for a sandbox run.
///
/// A floor, not a setting: [`build_options`] applies each only where the key is
/// unset, so a task-resolved credential-channel value wins; kept out of
/// `settings.json`, whose `env` block would outrank it.
pub const CLAUDE_ENV_DEFAULTS: &[(&str, &str)] = &[
    ("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "300000"),
    ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1"),
    ("CLAUDE_CODE_SUBAGENT_MODEL", ClaudeModel::Sonnet.api_id()),
    // No telemetry leaves the pod. The first two also stop feature-flag
    // fetching, which holds every flag at its code default.
    ("DISABLE_TELEMETRY", "1"),
    ("DO_NOT_TRACK", "1"),
    ("DISABLE_ERROR_REPORTING", "1"),
    ("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1"),
    ("DISABLE_AUTOUPDATER", "1"),
    ("CLAUDE_CODE_AUTO_CONNECT_IDE", "false"),
    ("CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL", "1"),
    ("CLAUDE_CODE_DISABLE_OFFICIAL_MARKETPLACE_AUTOINSTALL", "1"),
    // Tools and commands this runtime cannot reach, or that the `moat` CLI
    // already covers, kept out of the agent's context.
    ("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "1"),
    ("CLAUDE_CODE_DISABLE_AGENT_VIEW", "1"),
    ("CLAUDE_CODE_DISABLE_ARTIFACT", "1"),
    ("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY", "1"),
    ("DISABLE_COST_WARNINGS", "1"),
    ("DISABLE_DOCTOR_COMMAND", "1"),
    ("DISABLE_EXTRA_USAGE_COMMAND", "1"),
    ("DISABLE_FEEDBACK_COMMAND", "1"),
    ("DISABLE_INSTALL_GITHUB_APP_COMMAND", "1"),
    ("DISABLE_LOGIN_COMMAND", "1"),
    ("DISABLE_LOGOUT_COMMAND", "1"),
    ("DISABLE_UPGRADE_COMMAND", "1"),
];

/// The `--model` value one model choice resolves to, or `None` when it names no
/// model and the next choice should be tried.
///
/// `default` is the catalog's "whatever this install runs" entry rather than a
/// model, so it names nothing; an alias resolves through
/// [`claude_model_id`] and anything else is already an id.
fn claude_model(choice: Option<String>) -> Option<String> {
    let model = choice.map(|m| m.trim().to_string())?;
    if model.is_empty() || model == "default" {
        return None;
    }
    Some(claude_model_id(&model).map_or(model, str::to_string))
}

/// Build `ClaudeOptions` from config + per-request overrides.
pub fn build_options(
    config: &Config,
    request: &AgentRunRequest,
    cancel_token: CancellationToken,
    agent_state: Arc<AgentState>,
    env: Option<HashMap<String, String>>,
) -> ClaudeOptions {
    // Determine model: request > agentConfig > DEFAULT_CLAUDE_CODE_MODEL env.
    // Each choice resolves on its own, so a task carrying the catalog's
    // `default` falls through to the install's rather than ending the chain.
    let model = [
        request.model.clone(),
        config.agent_config.model.clone(),
        config.default_model.clone(),
    ]
    .into_iter()
    .find_map(claude_model);

    let system_prompt = build_system_prompt(config, request);

    let plugins = build_plugins(config);

    let is_plan_mode = request.agent_mode.as_deref() == Some("plan");

    // Build the permission handler for interactive prompts (AskUserQuestion,
    // ExitPlanMode). It also enforces plan-mode write blocking when active.
    let permission_handler =
        InteractivePermissionHandler::with_plan_mode(agent_state, is_plan_mode);

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
        // Subprocess mode: each turn is one-shot `claude -p`. The CLI's
        // `run_in_background` Bash dies with the process at turn end (see
        // https://code.claude.com/docs/en/headless), so disable it — a
        // genuinely detached process (`setsid`/`nohup … &`) survives and is
        // covered by the post-turn background-completion watch instead.
        let mut env = env.unwrap_or_default();
        env.entry("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS".to_string())
            .or_insert_with(|| "1".to_string());
        Some(env)
    };

    // Apply the CLI defaults last, and only where the key is still unset, so a
    // scoped value delivered on the credential channel outranks them.
    let env = env.map(|mut env| {
        for (key, value) in CLAUDE_ENV_DEFAULTS {
            env.entry((*key).to_string())
                .or_insert_with(|| (*value).to_string());
        }
        // The CLI reads this one as a model, so an alias delivered here fails a
        // subagent turn the way `--model opus` fails the root one.
        if let Some(subagent) = env.get_mut("CLAUDE_CODE_SUBAGENT_MODEL") {
            let model = subagent.trim();
            *subagent = claude_model_id(model).unwrap_or(model).to_string();
        }
        env
    });

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
        // Plan mode always uses Plan so InteractivePermissionHandler can
        // block writes until ExitPlanMode. Otherwise uses the configured
        // mode (`SANDBOX_PERMISSION_MODE`, default BypassPermissions):
        // AcceptEdits approves only file edit/write, leaving WebSearch/
        // WebFetch denied — the CLI never routes those through this handler.
        permission_mode: Some(if is_plan_mode {
            PermissionMode::Plan
        } else {
            config.permission_mode.clone()
        }),
        cwd: Some(config.workspace_dir.clone()),
        config_dir: Some(config.claude_config_dir.clone()),
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
        // See `disallowed_tools` for what's removed and why.
        disallowed_tools: Some(disallowed_tools(config, is_plan_mode)),
        ..ClaudeOptions::default()
    }
}

/// Tools removed entirely from the CLI's context for this run.
///
/// Plan mode keeps `ExitPlanMode`, the only way out. Monitor is disallowed
/// in subprocess mode: its watcher dies at turn end before events reach the
/// agent (tmux keeps the TUI alive, so it stays allowed there). Headless
/// interactive tools are disallowed since nobody is watching to answer them.
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

/// The `--effort` value for a requested effort, or `None` to leave the CLI on
/// its own default.
///
/// `ultracode` maps to `xhigh` (a Claude Code setting, not a CLI level); an
/// unknown level is dropped with a warning instead of passed through.
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
/// The sandbox prepends this note because only it knows the concrete
/// `workspace_dir`/`repo_dir` paths for this pod; the backend assembles the
/// rest without them.
pub fn build_prompt_append(config: &Config, request: &AgentRunRequest) -> String {
    let backend_prompt = request.system_prompt.clone().unwrap_or_default();
    let layout_note = build_workspace_layout_note(config);
    let mut sections = vec![layout_note];
    if let Some(note) = build_resource_limits_note(
        crate::limits::memory_limit_bytes(),
        config.workspace_limit.clone(),
    ) {
        sections.push(note);
    }
    if let Some(note) = build_managed_command_note(config) {
        sections.push(note);
    }
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
/// States what persists: `workspace_sync` snapshots everything under
/// `WORKSPACE_DIR` except gitignored paths inside a repo, so a sibling clone
/// and a loose scratch file survive a restart and anything outside the dir does
/// not. Collapses to one paragraph when `repo_dir == workspace_dir`.
fn build_workspace_layout_note(config: &Config) -> String {
    let workspace_dir = &config.workspace_dir;
    if config.repo_dir == config.workspace_dir {
        format!(
            "## Workspace layout\n\
             Your working directory is `{workspace_dir}`, which holds the task's git \
             repository. Committed history plus your uncommitted, non-gitignored \
             working-tree changes are snapshotted to durable storage and survive \
             sandbox restarts. Build artifacts and other gitignored paths are not \
             snapshotted (they are regenerated by setup commands). The snapshot is \
             taken periodically and on turn boundaries, not on every write, so a \
             restart can return the tree as it was slightly before the pod died — \
             the restart note dates the snapshot when that happens."
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
        let others = if secondary.is_empty() {
            String::new()
        } else {
            format!(
                "- Also checked out beside it, as part of this workspace: {}. Work in \
                 them the same way; they are part of the task, not stray clones.\n",
                secondary.join(", ")
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
             Everything else under `{workspace_dir}` is snapshotted too — a repo you \
             clone yourself, and loose files and directories outside any repo — so \
             that is where scratch work belongs if it has to survive a restart. Two \
             things do not survive: gitignored paths inside a repo (build artifacts \
             and dependencies), and anything outside `{workspace_dir}`, including \
             `/tmp`, `~/.kube` and `moat cmd` logs. The snapshot is taken \
             periodically and on turn boundaries, not on every write, so a restart \
             can return the tree as it was slightly before the pod died — the \
             restart note dates the snapshot when that happens."
        )
    }
}

/// Name the two ceilings the pod runs inside, and the tools that read them.
///
/// The agent otherwise learns of a cap only by crossing it, and the first thing
/// it reaches for reports the node instead: `free` and `df` both answer for the
/// host, several times larger than either ceiling. `None` in a runtime that
/// caps neither.
fn build_resource_limits_note(memory: Option<u64>, workspace: Option<String>) -> Option<String> {
    if memory.is_none() && workspace.is_none() {
        return None;
    }
    let mut lines = vec!["## Resource limits".to_string()];
    if let Some(bytes) = memory {
        lines.push(format!(
            "- Memory is capped at {} for this whole container, not the much \
             larger figure `free` prints — that is the node's. \
             `$SANDBOX_MEMORY_LIMIT` holds the same cap in bytes and \
             `/sys/fs/cgroup/memory.peak` holds what your last build actually \
             used; lower the build's parallelism when the second approaches \
             the first. Raising `NODE_OPTIONS --max-old-space-size` past the \
             cap makes a kill likelier rather than less likely.",
            crate::limits::format_gib(bytes)
        ));
    }
    if let Some(limit) = workspace {
        lines.push(format!(
            "- Disk is capped at {limit} across the workspace volume, which \
             backs your working directory and `/tmp`, plus \
             `/opt/cargo/registry` and `/opt/cargo/git` when the cargo cache \
             is persisted — separate mount points charged to one limit, so \
             `du -sx` stops at each boundary and you add the figures up \
             (`/opt/cargo` is not one of them and measures nothing). \
             `df` reports the node's filesystem and its free space \
             tells you nothing about this limit. Filling the volume either \
             gets the pod evicted mid-command and replaced, which looks like \
             an unexplained restart, or fails a write with `No space left on \
             device` — so clear build output (`target/`, `node_modules/`, \
             caches) when the measurement nears the cap."
        ));
    }
    Some(lines.join("\n"))
}

/// Tell the agent to hand long-running work to `moat cmd`.
///
/// Subprocess-mode only: the per-turn CLI runs one-shot with background tasks
/// disabled, so an agent left to itself blocks the turn polling a build; tmux
/// mode keeps a persistent session, where this would mislead.
fn build_managed_command_note(config: &Config) -> Option<String> {
    if config.use_tmux {
        return None;
    }
    Some(
        "## Long-running commands\n\
         Your shell tool kills a long command — Claude Code's Bash tool at 2 \
         minutes unless you pass a longer `timeout`, and never past 10 minutes \
         whatever you pass — and this session cannot background one. For work \
         that outlives a single shell call — a slow build or test suite, a dev \
         server — start it with `moat cmd run --cwd <dir> \"<command>\"` \
         instead of blocking a turn on `sleep` polling. \
         `--cwd` defaults to your shell tool's current \
         directory, so pass it explicitly to pin the command to the directory \
         you meant rather than wherever an earlier `cd` left the shell. \
         The sandbox owns the process: it keeps the sandbox awake while the \
         command runs and wakes you the moment it finishes. End your turn once \
         it is started — that wake is guaranteed, including when the command \
         fails or hits its deadline, so do not sleep, poll, or schedule a \
         fallback wakeup of your own. To run something only after a command \
         finishes, such as a second build sharing its target directory, start \
         it when that wake arrives, or as `moat cmd run \"moat cmd wait <id> && \
         <next>\"`; never loop on `moat cmd status` output, which ends with \
         the command's log. Check on one with `moat cmd list` / `moat cmd \
         status <id>` / `moat cmd logs <id>`, and stop one with `moat cmd kill \
         <id>`. A command is killed at its deadline (4h by default); pass \
         `--timeout <seconds>` to lower it for something that should finish \
         fast, or raise it for a server meant to run the whole session."
            .to_string(),
    )
}

/// The root instruction filenames to look for, in preference order: Claude
/// reads `CLAUDE.md`, Codex reads `AGENTS.md`.
///
/// The agent is pointed at the file it would load itself; the other name is the
/// fallback, since a repo commits one as a symlink to the other.
fn repo_instruction_filenames(config: &Config) -> [&'static str; 2] {
    let is_codex = config
        .agent_profile
        .as_deref()
        .and_then(|raw| resolve_agent_harness(raw).ok())
        == Some(moatless_types::agent::Harness::Codex);
    if is_codex {
        ["AGENTS.md", "CLAUDE.md"]
    } else {
        ["CLAUDE.md", "AGENTS.md"]
    }
}

/// Cap on how much of one repo's instruction file is inlined. Generous enough
/// that a normal `CLAUDE.md` lands whole; the cut only protects the prompt from
/// a runaway file, and what is dropped is still reachable at the path shown.
const MAX_INLINED_INSTRUCTIONS_BYTES: usize = 64 * 1024;

/// Build the "# Repository Instructions" section for each checked-out
/// repo's root instruction file, inlined by default since a pointer alone
/// often goes unread.
///
/// A repo opts out via `frontloadInstructions: false`, governing the whole
/// section in a multi-repo workspace. Returns `None` when no repo has one.
fn build_repo_instructions_section(config: &Config) -> Option<String> {
    let filenames = repo_instruction_filenames(config);
    let paths: Vec<String> = config
        .repo_dirs()
        .iter()
        .filter_map(|dir| repo_instruction_path(dir, filenames))
        .collect();

    if paths.is_empty() {
        return None;
    }

    if !config.agent_config.frontload_instructions_enabled() {
        let listed: Vec<String> = paths.iter().map(|path| format!("- `{path}`")).collect();
        return Some(format!(
            "# Repository Instructions\n\n\
             Each repository cloned into this workspace carries its own instruction file. \
             Read the one for a repository before doing anything in it:\n{}",
            listed.join("\n")
        ));
    }

    let mut sections = vec![
        "# Repository Instructions\n\n\
         Each repository cloned into this workspace carries its own instruction file. \
         Each one is inlined below, under the path it came from — these are the \
         standing rules for that repository, already in front of you, so follow them \
         when working in it rather than re-reading the file."
            .to_string(),
    ];
    sections.extend(paths.iter().map(|path| inline_repo_instructions(path)));
    Some(sections.join("\n\n"))
}

/// Render one repo instruction file as a section titled by its path.
///
/// A read failure degrades to pointing at the file instead of inlining it.
fn inline_repo_instructions(path: &str) -> String {
    match read_capped(path, MAX_INLINED_INSTRUCTIONS_BYTES) {
        Ok((contents, truncated)) => {
            let body = contents.trim_end();
            if truncated {
                warn!(
                    path,
                    limit = MAX_INLINED_INSTRUCTIONS_BYTES,
                    "Repo instruction file exceeds the inline cap; truncating"
                );
                format!(
                    "## `{path}`\n\n{body}\n\n_Truncated at {MAX_INLINED_INSTRUCTIONS_BYTES} bytes — read `{path}` for the rest._"
                )
            } else {
                format!("## `{path}`\n\n{body}")
            }
        }
        Err(error) => {
            warn!(path, %error, "Failed to read repo instruction file; pointing at it instead");
            format!(
                "## `{path}`\n\nThis file could not be read while assembling the prompt. Read it yourself before working in this repository."
            )
        }
    }
}

/// Read at most `limit` bytes of `path`, stopping there instead of reading the
/// whole file first.
///
/// The cap must apply *while* reading: `path` is repo-controlled and may
/// symlink to something endless (`/dev/zero`), so reading whole and trimming
/// after would let a repo dictate assembly's memory and time.
fn read_capped(path: &str, limit: usize) -> std::io::Result<(String, bool)> {
    use std::io::Read;

    // One byte past the cap is what distinguishes "ends exactly at the cap" from
    // "there is more we are not showing".
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut buf)?;

    let truncated = buf.len() > limit;
    buf.truncate(limit);

    let valid_up_to = match std::str::from_utf8(&buf) {
        Ok(_) => buf.len(),
        Err(error) => error.valid_up_to(),
    };
    buf.truncate(valid_up_to);

    // Lossless: the buffer is now exactly the prefix `from_utf8` accepted.
    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
}

/// The path of a repo's root instruction file: the first of `filenames` that
/// exists. Existence follows symlinks (the `CLAUDE.md -> AGENTS.md`
/// convention), so either committed name resolves.
fn repo_instruction_path(dir: &str, filenames: [&'static str; 2]) -> Option<String> {
    filenames.into_iter().find_map(|filename| {
        let path = Path::new(dir).join(filename);
        path.exists().then(|| path.to_string_lossy().into_owned())
    })
}

/// Build plugin configs from the plugins cloned into the sandbox.
///
/// Each `path` becomes a `--plugin-dir`, which the CLI itself exports as
/// `${CLAUDE_PLUGIN_ROOT}` to that plugin's hooks and MCP servers and expands
/// in the skills its Skill tool loads; the SDK has no per-plugin env to set.
fn build_plugins(config: &Config) -> Vec<PluginConfig> {
    build_plugins_with_root(&config.plugins_dir())
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

/// Resolve session resume from request into `(resume_session_id,
/// continue_conversation)`.
///
/// `WithId(id)` maps to `--resume <id>`; `Last` maps to `--continue`, letting
/// the CLI find the latest session.
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

    use crate::config::test_support::minimal_config;
    use moatless_workspace_types::workspace::RepoCheckout;
    use std::fs;
    use tempfile::tempdir;

    fn test_config(use_tmux: bool) -> Config {
        Config {
            port: 0,
            grpc_port: 0,
            sandbox_id: "task-test".to_string(),
            task_id: "task-test".to_string(),
            repo_dir: "/tmp/workspace".to_string(),
            backend_grpc_url: "http://localhost:50051".to_string(),
            claude_cli_path: "/usr/bin/claude".to_string(),
            codex_cli_path: "/usr/bin/codex".to_string(),
            use_tmux,
            ..minimal_config()
        }
    }

    fn test_request() -> AgentRunRequest {
        AgentRunRequest {
            content: Vec::new(),
            model: None,
            system_prompt: None,
            resume: None,
            agent_profile: None,
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
        )
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    /// Plugins are read from under the workspace, where `workspace_sync`
    /// restores them after a pod replacement.
    #[test]
    fn build_plugins_finds_cloned_plugin_dir() {
        let root = tempdir().unwrap();
        let mut config = test_config(false);
        config.workspace_dir = root.path().to_string_lossy().into_owned();
        let direct = root.path().join(".plugins/direct-plugin");
        write(
            &direct.join(".claude-plugin/plugin.json"),
            r#"{"name":"direct-plugin"}"#,
        );

        let plugins = build_plugins(&config);

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
        );
        assert_eq!(options.permission_mode, Some(PermissionMode::AcceptEdits));
    }

    #[test]
    fn env_defaults_apply_when_the_task_resolves_none() {
        let options = build_test_options(false);
        let env = options.env.unwrap();
        for (key, value) in CLAUDE_ENV_DEFAULTS {
            assert_eq!(env.get(*key).map(String::as_str), Some(*value), "{key}");
        }
    }

    /// The point of the floor: a value resolved for the task outranks the
    /// default, so an install can re-enable what the sandbox turns off without
    /// a code change.
    #[test]
    fn a_resolved_value_outranks_the_env_default() {
        let resolved = HashMap::from([
            ("DISABLE_TELEMETRY".to_string(), "0".to_string()),
            (
                "CLAUDE_CODE_SUBAGENT_MODEL".to_string(),
                "claude-opus-5".to_string(),
            ),
        ]);
        let options = build_options(
            &test_config(false),
            &test_request(),
            CancellationToken::new(),
            Arc::new(AgentState::new()),
            Some(resolved),
        );

        let env = options.env.unwrap();
        assert_eq!(env.get("DISABLE_TELEMETRY").map(String::as_str), Some("0"));
        assert_eq!(
            env.get("CLAUDE_CODE_SUBAGENT_MODEL").map(String::as_str),
            Some("claude-opus-5")
        );
        // Untouched keys still get their default.
        assert_eq!(env.get("DO_NOT_TRACK").map(String::as_str), Some("1"));
    }

    /// What the picker offers is the alias; what leaves for the CLI is the id
    /// the proxy routes on, spelled out here rather than read back off the
    /// table that produced it.
    #[test]
    fn a_requested_alias_reaches_the_cli_as_a_model_id() {
        let expected = [
            ("opus", "claude-opus-5-5"),
            ("sonnet", "claude-sonnet-5"),
            ("haiku", "claude-haiku-4-5-20251001"),
            ("fable", "claude-fable-5-1"),
        ];
        for (alias, id) in expected {
            let mut request = test_request();
            request.model = Some(alias.to_string());
            let options = build_options(
                &test_config(false),
                &request,
                CancellationToken::new(),
                Arc::new(AgentState::new()),
                None,
            );
            assert_eq!(options.model.as_deref(), Some(id), "{alias}");
        }
    }

    /// A subagent model is an env key, not a `--model` argument, and the CLI
    /// reads it the same way.
    #[test]
    fn a_subagent_alias_delivered_as_env_reaches_the_cli_as_an_id() {
        let resolved = HashMap::from([(
            "CLAUDE_CODE_SUBAGENT_MODEL".to_string(),
            "haiku".to_string(),
        )]);
        let options = build_options(
            &test_config(false),
            &test_request(),
            CancellationToken::new(),
            Arc::new(AgentState::new()),
            Some(resolved),
        );

        assert_eq!(
            options.env.unwrap().get("CLAUDE_CODE_SUBAGENT_MODEL"),
            Some(&"claude-haiku-4-5-20251001".to_string())
        );
    }

    /// A task left on the catalog's `default` names no model, so the install's
    /// own default still decides — the choice does not end the chain.
    #[test]
    fn the_default_entry_falls_through_to_the_install_default() {
        let mut config = test_config(false);
        config.default_model = Some("claude-opus-5".to_string());
        let mut request = test_request();
        request.model = Some("default".to_string());
        let options = build_options(
            &config,
            &request,
            CancellationToken::new(),
            Arc::new(AgentState::new()),
            None,
        );

        assert_eq!(options.model.as_deref(), Some("claude-opus-5"));
    }

    #[test]
    fn claude_model_passes_an_id_through_and_drops_the_default_entry() {
        assert_eq!(
            claude_model(Some("claude-opus-4-6".to_string())).as_deref(),
            Some("claude-opus-4-6")
        );
        assert_eq!(
            claude_model(Some("  opus ".to_string())).as_deref(),
            Some("claude-opus-5-5")
        );
        assert_eq!(claude_model(Some("default".to_string())), None);
        assert_eq!(claude_model(Some("   ".to_string())), None);
        assert_eq!(claude_model(None), None);
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
        // `workspace_sync` snapshots the whole dir, so scratch work beside the
        // repo survives and the note must not send the agent to `/tmp` instead.
        assert!(
            note.contains("Everything else under `/opt/moatless/workspace` is snapshotted too"),
            "{note}"
        );
        assert!(note.contains("`/tmp`"), "{note}");
        // A single-repo workspace has nothing else to list.
        assert!(!note.contains("Also checked out beside it"));
    }

    /// A multi-repo workspace must tell the agent the sibling checkouts are
    /// part of the task, not stray clones it should ignore.
    #[test]
    fn layout_note_lists_the_workspaces_other_repos() {
        let mut config = test_config(false);
        config.workspace_dir = "/opt/moatless/workspace".to_string();
        config.repo_dir = "/opt/moatless/workspace/api".to_string();
        config.repos = vec![
            RepoCheckout {
                uses_ssh: false,
                clone_url: "https://github.com/acme/api.git".into(),
                repository_id: String::new(),
                branch: None,
                provider: "github".into(),
                mount_name: "api".into(),
                is_primary: true,
            },
            RepoCheckout {
                uses_ssh: false,
                clone_url: "https://github.com/acme/web.git".into(),
                repository_id: String::new(),
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
        assert!(
            note.contains("Everything else under `/opt/moatless/workspace` is snapshotted too"),
            "{note}"
        );
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

    /// Both ceilings, and the tool that reads each — the agent's first instinct
    /// on an OOM is to raise the heap, and on a disk question to run `df`.
    #[test]
    fn the_limits_note_names_each_cap_and_how_to_read_it() {
        let note = build_resource_limits_note(Some(8 * 1024 * 1024 * 1024), Some("20Gi".into()))
            .expect("a note when either limit is known");
        assert!(note.contains("capped at 8 GiB"), "{note}");
        assert!(note.contains("$SANDBOX_MEMORY_LIMIT"), "{note}");
        assert!(note.contains("/sys/fs/cgroup/memory.peak"), "{note}");
        assert!(note.contains("makes a kill likelier"), "{note}");
        assert!(note.contains("capped at 20Gi"), "{note}");
        assert!(note.contains("du -sx"), "{note}");
        assert!(note.contains("No space left on device"), "{note}");
    }

    #[test]
    fn a_note_needs_one_cap_and_an_uncapped_runtime_gets_none() {
        assert_eq!(build_resource_limits_note(None, None), None);
        assert!(build_resource_limits_note(None, Some("20Gi".into())).is_some());
        assert!(build_resource_limits_note(Some(8 * 1024 * 1024 * 1024), None).is_some());
    }

    #[test]
    fn managed_command_note_present_in_subprocess_mode_only() {
        let subprocess = build_prompt_append(&test_config(false), &test_request());
        assert!(subprocess.contains("## Long-running commands"));
        assert!(subprocess.contains("moat cmd run --cwd <dir>"));
        assert!(subprocess.contains("that wake is guaranteed"));

        let tmux = build_prompt_append(&test_config(true), &test_request());
        assert!(!tmux.contains("## Long-running commands"));
    }

    fn repo(mount_name: &str, is_primary: bool) -> RepoCheckout {
        RepoCheckout {
            uses_ssh: false,
            clone_url: "https://example.com/repo.git".into(),
            repository_id: String::new(),
            branch: None,
            provider: "github".into(),
            mount_name: mount_name.into(),
            is_primary,
        }
    }

    /// By default each repo's root instruction file is inlined under its path;
    /// a repo carrying only the other filename is picked up via the fallback.
    #[test]
    fn repo_instructions_section_inlines_each_repos_root_file_by_default() {
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
        assert!(append.contains(&format!("## `{api_dir}/CLAUDE.md`")));
        // Only AGENTS.md exists here, so the fallback name is what gets used.
        assert!(append.contains(&format!("## `{web_dir}/AGENTS.md`")));
        assert!(append.contains("API RULES"));
        assert!(append.contains("WEB RULES"));

        // Order: layout note, then repo instructions, then the backend prompt.
        let layout_idx = append.find("## Workspace layout").unwrap();
        let repo_idx = append.find("# Repository Instructions").unwrap();
        let backend_idx = append.find("BACKEND-PROMPT").unwrap();
        assert!(layout_idx < repo_idx && repo_idx < backend_idx);
    }

    /// `frontloadInstructions: false` reverts to listing paths, leaving the
    /// contents out of the prompt.
    #[test]
    fn repo_instructions_section_lists_paths_when_frontloading_is_disabled() {
        let root = tempdir().unwrap();
        let ws = root.path().to_string_lossy().to_string();
        let api_dir = format!("{ws}/api");
        write(&PathBuf::from(&api_dir).join("CLAUDE.md"), "API RULES");

        let mut config = test_config(false);
        config.workspace_dir = ws;
        config.repo_dir = api_dir.clone();
        config.repos = vec![repo("api", true)];
        config.agent_config.frontload_instructions = Some(false);

        let append = build_prompt_append(&config, &test_request());

        assert!(append.contains("Read the one for a repository before doing anything in it:"));
        assert!(append.contains(&format!("- `{api_dir}/CLAUDE.md`")));
        assert!(!append.contains("API RULES"));
    }

    /// An oversized instruction file is cut at the cap and the agent is told
    /// where to read the remainder.
    #[test]
    fn inlined_repo_instructions_are_truncated_at_the_cap() {
        let root = tempdir().unwrap();
        let ws = root.path().to_string_lossy().to_string();
        let api_dir = format!("{ws}/api");
        let body = "x".repeat(MAX_INLINED_INSTRUCTIONS_BYTES + 500);
        write(&PathBuf::from(&api_dir).join("CLAUDE.md"), &body);

        let mut config = test_config(false);
        config.workspace_dir = ws;
        config.repo_dir = api_dir.clone();
        config.repos = vec![repo("api", true)];

        let append = build_prompt_append(&config, &test_request());

        assert!(append.contains("_Truncated at"));
        assert!(append.contains(&format!("read `{api_dir}/CLAUDE.md` for the rest._")));
        assert!(append.len() < body.len() + 4096);
    }

    /// The cap cuts on a UTF-8 boundary, and a file within it is returned
    /// whole.
    #[test]
    fn read_capped_stops_at_the_limit_without_splitting_a_character() {
        let root = tempdir().unwrap();
        let path = root.path().join("CLAUDE.md");
        write(&path, "ééé"); // 2 bytes each
        let path = path.to_string_lossy().to_string();

        // An odd limit lands mid-character, so the cut backs off to 2 bytes.
        let (text, truncated) = read_capped(&path, 3).unwrap();
        assert_eq!(text, "é");
        assert!(truncated);

        // Exactly at the limit is not "there is more".
        let (text, truncated) = read_capped(&path, 6).unwrap();
        assert_eq!(text, "ééé");
        assert!(!truncated);

        let (text, truncated) = read_capped(&path, 64).unwrap();
        assert_eq!(text, "ééé");
        assert!(!truncated);
    }

    /// The cap has to bound the read itself. `/dev/zero` never ends, so this
    /// only terminates if the limit is applied during I/O rather than after —
    /// the case a repo could reach by symlinking its `CLAUDE.md`.
    #[cfg(unix)]
    #[test]
    fn read_capped_terminates_on_a_non_terminating_file() {
        let (text, truncated) = read_capped("/dev/zero", 128).unwrap();
        assert_eq!(text.len(), 128);
        assert!(truncated);
    }

    /// An unreadable path degrades to pointing at the file rather than failing
    /// the whole prompt.
    #[test]
    fn inline_repo_instructions_points_at_a_file_it_cannot_read() {
        let root = tempdir().unwrap();
        let missing = root
            .path()
            .join("nope/CLAUDE.md")
            .to_string_lossy()
            .to_string();
        let section = inline_repo_instructions(&missing);
        assert!(section.contains(&format!("## `{missing}`")));
        assert!(section.contains("could not be read"));
    }

    /// A repo committing both names is pointed at under the one the running
    /// agent would have loaded itself: `CLAUDE.md` for Claude, `AGENTS.md` for
    /// Codex.
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

        config.agent_profile = Some("claude-code".to_string());
        let claude = build_prompt_append(&config, &test_request());
        assert!(claude.contains(&format!("## `{dir}/CLAUDE.md`")));
        assert!(!claude.contains(&format!("## `{dir}/AGENTS.md`")));
        assert!(claude.contains("CLAUDE COPY"));
        assert!(!claude.contains("AGENTS COPY"));

        config.agent_profile = Some("codex".to_string());
        let codex = build_prompt_append(&config, &test_request());
        assert!(codex.contains(&format!("## `{dir}/AGENTS.md`")));
        assert!(!codex.contains(&format!("## `{dir}/CLAUDE.md`")));
        assert!(codex.contains("AGENTS COPY"));
        assert!(!codex.contains("CLAUDE COPY"));
    }

    /// Claude's transport/model variants are the same harness, and an unset or
    /// unknown type is not Codex — all keep the `CLAUDE.md` preference.
    #[test]
    fn repo_instruction_filenames_default_to_claude() {
        let mut config = test_config(false);
        for agent_profile in [
            None,
            Some("claude-code-tui"),
            Some("claude-code-fable"),
            Some("gpt-9000"),
        ] {
            config.agent_profile = agent_profile.map(str::to_string);
            assert_eq!(
                repo_instruction_filenames(&config),
                ["CLAUDE.md", "AGENTS.md"],
                "agent_profile={agent_profile:?}"
            );
        }
    }

    /// A repo with no root instruction file adds no section rather than an
    /// empty heading.
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
