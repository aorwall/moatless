//! Sandbox initialization orchestrator.
//!
//! Handles the full lifecycle of workspace setup in two phases:
//!
//! **Phase 1 (core init)**: Credential fetching, git authentication,
//! repository cloning, plugin sync. Health endpoint returns 503 during
//! this phase. On completion, `InitPhase::Ready` is set and health returns 200.
//!
//! **Phase 2 (workspace setup)**: Runs after health is ready. Copies
//! prebuilt deps, sets up build cache, runs setup commands.
//! These are non-fatal — failures are logged but don't affect health.

mod claude_code_settings;
mod codex_compat;
mod codex_config;
pub(crate) mod codex_watcher;
pub(crate) mod credentials;
mod dirs;
pub(crate) mod gh_hosts;
mod git;
mod plugin_skills;
pub(crate) mod refresher;
pub(crate) mod server_env;
pub(crate) mod transcript_sync;
mod workspace;
pub(crate) mod workspace_sync;

use moatless_events::SandboxSubType;
use sandbox_proto::PublishEventRequest;
use tracing::{error, info, warn};

use crate::state::SandboxState;

/// Init error with permanence classification.
/// Permanent errors use exit code 78 so the pod watcher stops immediately.
/// Transient errors use exit code 1 and let K8s retry.
pub(crate) enum InitError {
    /// Will never succeed on retry (repo not found, access denied).
    Permanent(String),
    /// Might succeed on retry (backend unreachable, network timeout).
    Transient(String),
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Permanent(msg) | Self::Transient(msg) => f.write_str(msg),
        }
    }
}

/// Tracks the current initialization phase.
///
/// Stored in `SandboxState` as `Arc<RwLock<InitPhase>>` and read by:
/// - HTTP `/health` endpoint (returns 503 until Ready)
/// - gRPC `CheckHealth` (returns healthy=true only when Ready)
/// - Message consumer (waits for Ready before polling)
#[derive(Debug, Clone)]
pub enum InitPhase {
    Starting,
    FetchingCredentials,
    CloningRepository,
    Ready,
    Failed(String),
}

/// Publish a sandbox init event via gRPC.
///
/// These events appear in the frontend's StartupTimeline alongside K8s events.
/// Uses `EventType::Sandbox` with `SandboxSubType::Initializing` — the sandbox
/// IS initializing while these events fire. Detailed info goes in the payload.
async fn publish_sandbox_event(
    state: &SandboxState,
    reason: &str,
    message: &str,
    is_warning: bool,
) {
    let payload = serde_json::json!({
        "reason": reason,
        "message": message,
        "eventType": if is_warning { "Warning" } else { "Normal" },
    });

    let payload_json = match serde_json::to_string(&payload) {
        Ok(json) => json,
        Err(e) => {
            warn!(error = %e, "Failed to serialize sandbox event payload");
            return;
        }
    };

    let proto_event_type: sandbox_proto::EventType = moatless_events::EventType::Sandbox.into();

    let req = PublishEventRequest {
        task_id: state.config.task_id.clone(),
        event_type: proto_event_type as i32,
        sub_type: SandboxSubType::Initializing.as_str().to_string(),
        payload_json,
    };

    state.events.client().publish_event(req).await;
}

/// Run the full initialization sequence (phase 1 + phase 2).
///
/// Called from `main.rs` via `tokio::spawn`. Phase 1 runs first and sets
/// `InitPhase::Ready` on success (or `Failed` on error). Phase 2 runs
/// immediately after Ready, performing workspace setup that doesn't need
/// to block the health endpoint.
pub async fn run_init(state: SandboxState) {
    info!(task_id = %state.config.task_id, "Starting sandbox initialization");

    // Phase 1: Core init — must complete before health returns 200
    if let Err(e) = run_core_init(&state).await {
        error!(error = %e, "Core initialization failed");
        // Write termination message for K8s to surface in pod status.
        let _ = std::fs::write("/dev/termination-log", e.to_string());
        let exit_code = match &e {
            InitError::Permanent(_) => 78, // EX_CONFIG — pod watcher stops immediately
            InitError::Transient(_) => 1,  // K8s retries, threshold handles it
        };
        std::process::exit(exit_code);
    }

    // Mark as ready — health endpoint now returns 200
    *state.init_phase.write().await = InitPhase::Ready;

    publish_sandbox_event(
        &state,
        "SandboxReady",
        "Core initialization complete, starting workspace setup",
        false,
    )
    .await;

    info!(task_id = %state.config.task_id, "Core init complete — health is now ready");

    // Fetch pending messages first so a new user message supersedes any
    // interrupted-turn replay. When pending messages exist we skip recovery and
    // let the dispatcher resume the prior session via `--resume <session_id>`
    // with the new user text as the prompt. When none exist we fall back to
    // replaying the interrupted turn.
    let task_id = state.config.task_id.clone();
    let has_pending_messages = match state.backend_client.get_pending_messages(&task_id).await {
        Ok(msgs) => !msgs.is_empty(),
        Err(e) => {
            warn!(error = %e, task_id = %task_id, "Failed to probe pending messages on startup");
            false
        }
    };

    if has_pending_messages {
        info!(
            task_id = %task_id,
            "Pending messages present on startup; skipping interrupted-run replay"
        );
    } else {
        recover_interrupted_run(&state).await;
    }

    // Fetch any messages that arrived during initialization.
    // Spawned (not awaited) so it doesn't block Phase 2 workspace setup.
    let startup_state = state.clone();
    tokio::spawn(async move {
        crate::agent::dispatcher::dispatch_pending_messages(startup_state).await;
    });

    // Phase 2: Workspace setup — runs after health is 200
    run_workspace_setup(&state).await;

    // Write workspace-ready sentinel AFTER setup commands complete (e.g. bun install).
    // Server containers poll for this file before starting, so they must wait
    // until dependencies are installed.
    let ready_path = format!("{}/.workspace-ready", state.config.data_dir);
    if let Err(e) = tokio::fs::write(&ready_path, "ready").await {
        warn!(error = %e, "Failed to write workspace-ready sentinel");
    }

    info!(task_id = %state.config.task_id, "Workspace setup complete");

    // Spawn the periodic credential refresher. GitHub App installation tokens
    // expire after 1h and user access tokens after 8h — long-running tasks
    // would otherwise fail mid-turn on `git push` / `gh pr comment`.
    // See `refresher.rs` for the full rationale.
    let refresher_state = state.clone();
    let refresher_cancel = state.lifecycle_cancel.clone();
    let refresher_interval = refresher::read_interval_from_env();
    tokio::spawn(refresher::run_credential_refresher(
        refresher_state,
        refresher_cancel,
        refresher_interval,
    ));

    // Spawn the periodic transcript uploader. Every interval it tars
    // `projects/<dashed-cwd>/` and ships it to S3 via the backend, so transcripts
    // persist off the per-task PVC (and are the *only* durability for the
    // ephemeral tier). `TRANSCRIPT_SYNC_INTERVAL_SECS=0` disables the loop;
    // sync-on-shutdown still flushes the final delta.
    if let Some(interval) = transcript_sync::read_interval_from_env() {
        let transcript_state = state.clone();
        let transcript_cancel = state.lifecycle_cancel.clone();
        tokio::spawn(transcript_sync::run_transcript_sync(
            transcript_state,
            transcript_cancel,
            interval,
        ));
    }

    // Spawn the periodic workspace uploader for the `s3` storage backend. Every
    // interval it tars the git working tree and PUTs it directly to S3 via a
    // backend-minted presigned URL, so the working tree survives the node-local
    // emptyDir across stop/start. `WORKSPACE_SYNC_INTERVAL_SECS=0` disables the
    // loop; sync-on-shutdown still flushes the final delta.
    if state.config.workspace_sync_enabled {
        if let Some(interval) = workspace_sync::read_interval_from_env() {
            let workspace_state = state.clone();
            let workspace_cancel = state.lifecycle_cancel.clone();
            tokio::spawn(workspace_sync::run_workspace_sync(
                workspace_state,
                workspace_cancel,
                interval,
            ));
        }
    }

    // Watch ~/.codex/auth.json for rotations. Codex rewrites this file every
    // time it refreshes its OAuth token; the watcher pushes the rotated blob
    // back to the backend so subsequent tasks (and pod restarts) see the new
    // refresh_token instead of the now-invalidated stored copy.
    spawn_codex_auth_watcher(&state).await;
}

/// Detect any interrupted agent run via the backend RPC and queue a resume on
/// the agent actor. Runs once on startup, **only when there are no new pending
/// user messages** — a fresh message supersedes the interrupted turn and
/// `dispatch_pending_messages` carries it forward instead.
///
/// The backend handler does the heavy lifting (looks up the unfinished
/// user-turn, increments `recovery_attempts`, publishes the
/// `agent.recovery_started` event). If we get back a turn we feed it through
/// the existing `--resume <session_id>` path; otherwise this is a no-op.
async fn recover_interrupted_run(state: &SandboxState) {
    let task_id = &state.config.task_id;
    match state.backend_client.recover_interrupted_run(task_id).await {
        Ok(Some(turn)) => {
            tracing::warn!(
                task_id = %task_id,
                session_id = %turn.session_id,
                turn_number = turn.turn_number,
                "Resuming interrupted agent run"
            );
            state.agent_handle.recover(
                turn.prompt_text,
                turn.agent_type,
                turn.agent_mode,
                turn.session_id,
            );
        }
        Ok(None) => {
            tracing::debug!(task_id = %task_id, "No interrupted run to recover");
        }
        Err(e) => {
            tracing::warn!(error = %e, task_id = %task_id, "recover_interrupted_run failed");
        }
    }
}

async fn spawn_codex_auth_watcher(state: &SandboxState) {
    let auth_path = std::path::PathBuf::from(format!("{}/auth.json", state.config.codex_home_dir));

    // Capture the current on-disk value as the baseline so the first rotation
    // (which produces a different blob) triggers the sync. If the file is
    // absent — Codex isn't configured for this task — the watcher still runs
    // and will pick up the file once it appears.
    let initial_value = tokio::fs::read_to_string(&auth_path)
        .await
        .unwrap_or_default();

    let handle = codex_watcher::spawn(
        state.config.task_id.clone(),
        auth_path,
        initial_value,
        std::sync::Arc::clone(&state.backend_client),
        state.lifecycle_cancel.clone(),
    );

    // Stash the handle so the credential dispatcher can seed the watcher's
    // cache when a backend-driven push lands — keeps backend writes from
    // echoing back as fresh `UpdateCredential` calls.
    *state.codex_watcher_handle.write().await = Some(handle);
}

/// Phase 1: Core initialization.
///
/// Performs essential steps that must complete before the pod is marked healthy:
/// 1. Ensure directories and symlinks
/// 2. Write the default Claude Code settings file (non-fatal)
/// 3. Fetch and apply credentials (fatal on failure)
/// 4. Configure git authentication (non-fatal)
/// 5. Clone repository (fatal on failure)
/// 6. Sync plugins (non-fatal)
async fn run_core_init(state: &SandboxState) -> Result<(), InitError> {
    // 1. Ensure directories exist
    dirs::ensure_directories(&state.config);

    // 2. Write the default Claude Code settings file. Static defaults; safe to
    //    overwrite on every start. Non-fatal — Claude CLI still runs without it.
    claude_code_settings::write_claude_code_settings(&state.config.claude_config_dir).await;

    // 2b. Write the default Codex config.toml. Disables the broken
    //     `codex_apps` MCP server. Non-fatal — Codex still runs without it.
    codex_config::write_codex_config(&state.config.codex_home_dir).await;

    // 3. Fetch credentials from backend via gRPC
    {
        *state.init_phase.write().await = InitPhase::FetchingCredentials;
    }

    if let Err(e) = credentials::fetch_and_apply_credentials(state).await {
        let msg = format!("Failed to fetch credentials: {e}");
        publish_sandbox_event(state, "CredentialsFailed", &msg, true).await;
        return Err(InitError::Transient(msg));
    }
    publish_sandbox_event(
        state,
        "CredentialsFetched",
        "Fetched credentials from backend",
        false,
    )
    .await;

    // 3. Configure git authentication (non-fatal)
    if let Err(e) = git::configure_git(&state.config).await {
        warn!(error = %e, "Git configuration failed (non-fatal)");
    } else {
        publish_sandbox_event(
            state,
            "GitConfigured",
            "Configured git authentication",
            false,
        )
        .await;
    }

    // 4. Clone repository (fatal on failure)
    {
        *state.init_phase.write().await = InitPhase::CloningRepository;
    }

    if let Some(ref clone_url) = state.config.git_clone_url {
        if !clone_url.is_empty() {
            match git::clone_repository(&state.config).await {
                Ok(()) => {
                    let branch_info = state
                        .config
                        .git_branch
                        .as_deref()
                        .map(|b| format!(" ({b})"))
                        .unwrap_or_default();
                    publish_sandbox_event(
                        state,
                        "RepositoryCloned",
                        &format!("Cloned repository{branch_info}"),
                        false,
                    )
                    .await;
                }
                Err(e) => {
                    let msg = format!("Failed to clone repository: {e}");
                    publish_sandbox_event(state, "RepositoryCloneFailed", &msg, true).await;
                    return Err(InitError::Permanent(msg));
                }
            }
        }
    }

    // 5. Sync plugins (non-fatal)
    if !state.config.plugin_ids.is_empty() {
        match workspace::sync_plugins(&state.config).await {
            Ok(count) => {
                publish_sandbox_event(
                    state,
                    "PluginsSynced",
                    &format!("Synced {count} plugin(s)"),
                    false,
                )
                .await;
            }
            Err(e) => {
                warn!(error = %e, "Plugin sync failed (non-fatal)");
            }
        }
    }

    // 6. Clone plugin git repos (non-fatal)
    if !state.config.plugin_repos.is_empty() {
        let cloned = workspace::clone_plugin_repos(&state.config).await;
        if !cloned.is_empty() {
            publish_sandbox_event(
                state,
                "PluginReposCloned",
                &format!("Cloned {} plugin repo(s)", cloned.len()),
                false,
            )
            .await;
        }
    }

    // 7. Stage plugin-bundled skills so the Codex agent can discover them.
    // Runs unconditionally so we pick up skills from both `plugin_repos`
    // (cloned above) and `plugin_ids` (installed by sync_plugins.sh).
    let staged = plugin_skills::stage_plugin_skills_for_codex().await;
    if staged > 0 {
        publish_sandbox_event(
            state,
            "CodexPluginSkillsStaged",
            &format!("Staged {staged} plugin skill(s) for Codex"),
            false,
        )
        .await;
    }

    // 8. Mirror CLAUDE.md → AGENTS.md and .claude/skills → .agents/skills
    // throughout the workspace so Codex finds the same instructions and
    // skill bundles Claude does. Runs unconditionally — agent type is
    // decided per-message; symlinks are cheap, idempotent, and harmless
    // when Claude is the active agent.
    if !state.config.workspace_dir.is_empty() {
        let workspace = std::path::Path::new(&state.config.workspace_dir);
        let agents_md = codex_compat::mirror_claude_md_to_agents_md(workspace);
        let agents_skills = codex_compat::mirror_claude_skills_to_agents_skills(workspace);
        let total = agents_md + agents_skills;
        if total > 0 {
            publish_sandbox_event(
                state,
                "CodexCompatMirrored",
                &format!("Mirrored {agents_md} AGENTS.md and {agents_skills} skill(s) for Codex",),
                false,
            )
            .await;
        }
    }

    // 9. Restore the Claude transcript from S3 (via backend-proxied gRPC) into
    //    `projects/<dashed-cwd>/`. Runs here — after the clone, before the sandbox
    //    is marked Ready — so `recover_interrupted_run` and pending-message
    //    dispatch see restored `--resume` history. Tier-agnostic and best-effort:
    //    a missing transcript (or a disabled feature) is non-fatal.
    transcript_sync::download_on_start(state).await;

    // 10. For the `s3` storage backend, overlay the persisted git working tree
    //     from S3 onto the fresh clone (agent edits / uncommitted changes from
    //     the previous pod). Runs after the clone, before Ready, so the agent
    //     sees restored state. Only when the backend enabled workspace sync;
    //     best-effort and non-fatal.
    if state.config.workspace_sync_enabled {
        workspace_sync::download_on_start(state).await;
    }

    Ok(())
}

/// Phase 2: Workspace setup.
///
/// Runs after health is ready. All steps are non-fatal — failures are logged
/// and published as warning events but don't affect pod health.
///
/// Steps:
/// 1. Copy prebuilt dependencies (node_modules, etc.)
/// 2. Set up build cache
/// 3. Run setup commands
async fn run_workspace_setup(state: &SandboxState) {
    for cmd in &state.config.setup_commands {
        publish_sandbox_event(
            state,
            "SetupRunning",
            &format!("Running setup command: {cmd}"),
            false,
        )
        .await;
        if let Err(e) = workspace::run_setup_command(&state.config, cmd).await {
            let msg = format!("Setup command failed: {cmd}: {e}");
            warn!("{msg}");
            publish_sandbox_event(state, "SetupFailed", &msg, true).await;
            // Non-fatal — continue with remaining setup
        }
    }

    publish_sandbox_event(
        state,
        "WorkspaceSetupComplete",
        "Workspace setup complete",
        false,
    )
    .await;
}
