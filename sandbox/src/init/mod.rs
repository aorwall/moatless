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
mod ssh;
pub(crate) mod transcript_sync;

#[cfg(test)]
mod replacement_test;
mod workspace;
mod workspace_bridge;
pub(crate) mod workspace_sync;

use moatless_events::SandboxStatus;
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
/// Uses `EventType::Sandbox` with `SandboxStatus::Initializing` — the sandbox
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
        sub_type: SandboxStatus::Initializing.as_str().to_string(),
        payload_json,
        event_id: String::new(),
        sequence: 0,
        run_id: String::new(),
    };

    state.events.enqueue(req);
}

/// Run the full initialization sequence (phase 1 + phase 2).
///
/// Called from `main.rs` via `tokio::spawn`. Phase 1 runs first and sets
/// `InitPhase::Ready` on success (or `Failed` on error). Phase 2 runs
/// immediately after Ready, performing workspace setup that doesn't need
/// to block the health endpoint.
pub async fn run_init(state: SandboxState) {
    info!(task_id = %state.config.task_id, "Starting sandbox initialization");

    // Phase 1: Core init — must complete before health returns 200.
    // Returns whether the workspace was rebuilt from a fresh clone (ephemeral
    // pre-restart state lost) so we can warn a resuming agent.
    let workspace_origin = match run_core_init(&state).await {
        Ok(origin) => origin,
        Err(e) => {
            error!(error = %e, "Core initialization failed");
            // Write termination message for K8s to surface in pod status.
            let _ = std::fs::write("/dev/termination-log", e.to_string());
            let exit_code = match &e {
                InitError::Permanent(_) => 78, // EX_CONFIG — pod watcher stops immediately
                InitError::Transient(_) => 1,  // K8s retries, threshold handles it
            };
            std::process::exit(exit_code);
        }
    };

    // Decide interrupted-Run vs pending-Message precedence, open the readiness
    // gate, and perform the one authoritative pending dispatch — in that order.
    // Ordering is the invariant: the precedence probe must run while external
    // dispatch is still gated (see `open_for_work`).
    open_for_work(&state, workspace_origin).await;

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

    // Spawn the periodic CLI session-state uploader. Every interval it tars
    // Claude transcripts plus Codex rollout sessions and ships them to S3 via
    // the backend, so agent resume state persists off the per-task PVC (and is
    // the only CLI-session durability for the ephemeral tier).
    // `TRANSCRIPT_SYNC_INTERVAL_SECS=0` disables the loop; sync-on-shutdown
    // still flushes the final delta.
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

/// Cross the boundary from "restored but private" to "open for work", in the one
/// order that closes the startup precedence race:
///
/// 1. **Decide precedence while dispatch is still gated.** Session state was
///    restored to disk in `run_core_init` (before Ready). We probe for a pending
///    Message and, only when none is waiting, enqueue the interrupted-Run replay.
///    External gRPC dispatch is still a no-op here because init is not yet Ready.
/// 2. **Open the gate.** Flip `InitPhase::Ready` (health now returns 200) and
///    announce `SandboxReady`.
/// 3. **Dispatch once, authoritatively.** Drain any pending Message — a no-op if
///    step 1 already enqueued a Recover, otherwise the Message that superseded it.
///
/// Doing step 1 before step 2 is what prevents a backend notification that races
/// init from fetching, submitting, and acknowledging the newer pending Message in
/// the window before the probe — which would leave the probe seeing no pending
/// row and enqueueing a now-stale Recover behind the Message.
pub(crate) async fn open_for_work(state: &SandboxState, workspace_origin: WorkspaceOrigin) {
    recover_interrupted_run_unless_pending(state, workspace_origin).await;

    *state.init_phase.write().await = InitPhase::Ready;
    publish_sandbox_event(
        state,
        "SandboxReady",
        "Core initialization complete, starting workspace setup",
        false,
    )
    .await;
    info!(task_id = %state.config.task_id, "Core init complete — health is now ready");

    // Awaited, not detached: dispatch only fetches, submits to the agent actor,
    // and returns (the turn itself runs in the coordinator) — it does not block on
    // the turn, so Phase 2 workspace setup is not delayed. Awaiting keeps this the
    // one authoritative startup dispatch and avoids an orphaned JoinHandle.
    crate::agent::dispatcher::dispatch_pending_messages(state.clone()).await;
}

/// How the workspace came to exist on this start — determines what (if anything)
/// a resuming agent must be warned lost across the restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceOrigin {
    /// Repo persisted in place (EBS/PVC volume already held it) or this is a
    /// fresh task — nothing was lost, so no restart note is needed.
    Persisted,
    /// Restored byte-for-byte from an S3 snapshot (the `s3` storage backend).
    /// The git tree — committed history plus uncommitted, non-gitignored changes
    /// — and out-of-repo files are intact, but gitignored build artifacts inside
    /// repos were NOT in the snapshot and are gone.
    RestoredFromSnapshot,
    /// Rebuilt from a fresh `git clone` — all ephemeral pre-restart state (incl.
    /// unpushed commits and uncommitted changes) may be gone.
    Recloned,
}

impl WorkspaceOrigin {
    /// The `<sandbox-restart-note>` to prepend to a resumed turn, or `None` when
    /// nothing was lost and the agent needs no warning.
    fn restart_note(self, workspace_dir: &str) -> Option<String> {
        match self {
            Self::Persisted => None,
            Self::RestoredFromSnapshot => Some(workspace_restored_note(workspace_dir)),
            Self::Recloned => Some(workspace_recloned_note(workspace_dir)),
        }
    }
}

/// Note prepended to a resumed turn when the workspace was restored from an S3
/// snapshot across a restart (e.g. the pod was replaced, or evicted for exceeding
/// its workspace volume size limit). The git tree is intact, but gitignored build
/// artifacts are not part of the snapshot, so they were wiped and must be rebuilt.
fn workspace_restored_note(workspace_dir: &str) -> String {
    format!(
        "<sandbox-restart-note>\n\
         This sandbox was restarted — the pod was replaced or evicted (a common \
         cause is the workspace volume exceeding its size limit, e.g. from large \
         build outputs) — and the repository at {workspace_dir} was restored from \
         a snapshot. Your committed history and uncommitted, non-gitignored \
         changes are intact and present. However, gitignored build artifacts and \
         dependencies inside git repos (e.g. target, node_modules, dist, .venv) \
         were NOT part of the snapshot and have been wiped. Re-run install/build \
         steps (e.g. `cargo build`, `bun install`) before relying on compiled \
         output. To avoid another eviction, keep build artifacts from growing \
         without bound — clean stale targets if the workspace gets large.\n\
         </sandbox-restart-note>\n\n"
    )
}

/// Note prepended to a resumed turn when the workspace was rebuilt from a fresh
/// clone across the restart, so the agent knows its pre-restart scratch state is
/// gone rather than silently working against a reset tree.
fn workspace_recloned_note(workspace_dir: &str) -> String {
    format!(
        "<sandbox-restart-note>\n\
         This sandbox was restarted (the pod was replaced or evicted) and the \
         repository at {workspace_dir} was rebuilt from a fresh `git clone`. \
         Committed, pushed git content is present, but ephemeral state from \
         before the restart is gone: gitignored dependencies and build artifacts \
         (e.g. node_modules, target, dist, .venv), and any files created outside \
         the git repository. Any local commits or uncommitted changes that were \
         never pushed may also be missing. Re-run install/build steps and verify \
         your working state before continuing.\n\
         </sandbox-restart-note>\n\n"
    )
}

/// Interrupted-run precedence gate: probe for pending user messages first, and
/// replay the interrupted run **only** when none are waiting. A fresh user
/// message supersedes the interrupted turn — `dispatch_pending_messages` resumes
/// the prior session with the new text as the prompt instead. A probe error
/// **fails closed** — treated as "pending present", skipping the replay — so an
/// unknown probe result can't resume the stale Run only to have a newer Message
/// dispatched concurrently once the backend recovers.
///
/// `pub(crate)` so the Sandbox-replacement lifecycle can be exercised in-crate
/// with an in-memory backend without driving the whole of `run_init`.
pub(crate) async fn recover_interrupted_run_unless_pending(
    state: &SandboxState,
    origin: WorkspaceOrigin,
) {
    let task_id = &state.config.task_id;
    // Fail closed on a probe error: treat "unknown" as "pending present" and skip
    // the interrupted-run replay. Replaying on an unknown probe result risks
    // resuming the stale Run and then having a newer Message dispatched
    // concurrently once the backend recovers — two turns racing the same pod.
    let has_pending_messages = match state.backend_client.get_pending_messages(task_id).await {
        Ok(msgs) => !msgs.is_empty(),
        Err(e) => {
            warn!(
                error = %e,
                task_id = %task_id,
                "Failed to probe pending messages on startup; skipping interrupted-run replay"
            );
            true
        }
    };

    if has_pending_messages {
        info!(
            task_id = %task_id,
            "Pending messages present (or probe failed) on startup; skipping interrupted-run replay"
        );
    } else {
        recover_interrupted_run(state, origin).await;
    }
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
///
/// When `origin` indicates the workspace was rebuilt from a fresh clone or
/// restored from an S3 snapshot on this restart, a `<sandbox-restart-note>` is
/// prepended to the resumed prompt telling the agent exactly what pre-restart
/// state (unpushed commits, or just gitignored build artifacts) is gone.
async fn recover_interrupted_run(state: &SandboxState, origin: WorkspaceOrigin) {
    let task_id = &state.config.task_id;
    match state.backend_client.recover_interrupted_run(task_id).await {
        Ok(Some(turn)) => {
            tracing::warn!(
                task_id = %task_id,
                session_id = %turn.session_id,
                turn_number = turn.turn_number,
                workspace_origin = ?origin,
                "Resuming interrupted agent run"
            );
            // Prepend a restart note so the resuming agent doesn't assume its
            // prior scratch state survived. Which note depends on how the
            // workspace was reconstituted (see `WorkspaceOrigin`).
            let prompt_text = match origin.restart_note(&state.config.repo_dir) {
                Some(note) => format!("{note}{}", turn.prompt_text),
                None => turn.prompt_text,
            };
            state.agent_handle.recover(
                prompt_text,
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
///
/// Runs Phase 1 core init. Returns how the workspace was reconstituted (see
/// [`WorkspaceOrigin`]) so the caller can warn a resuming agent about exactly
/// what pre-restart state was lost — a fresh clone loses everything ephemeral,
/// an S3 snapshot restore loses only gitignored build artifacts, and a persisted
/// volume loses nothing.
async fn run_core_init(state: &SandboxState) -> Result<WorkspaceOrigin, InitError> {
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
        // An expired bootstrap token is unrecoverable without re-provisioning
        // (a pod restart presents the same expired env), so surface it as a
        // permanent config failure (exit 78) instead of a transient crash loop.
        return Err(match e {
            credentials::CredentialFetchError::Permanent(_) => InitError::Permanent(msg),
            credentials::CredentialFetchError::Transient(_) => InitError::Transient(msg),
        });
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

    // 4. For the `s3` storage backend, try to restore a full-repo snapshot from
    //    S3 *before* cloning. A snapshot that carries `.git` is a complete repo
    //    at the exact prior HEAD + index + dirty working tree, so restoring it
    //    lets us skip the clone entirely — no fresh clone of an advanced base
    //    branch to drift against (which previously surfaced upstream changes as
    //    phantom uncommitted "reverts"). Returns false (leaving the workspace
    //    empty) for a fresh task, a disabled feature, a failed transfer, or a
    //    legacy working-tree-only snapshot — in which case we clone below.
    let restored_full_repo = if state.config.workspace_sync_enabled {
        let restored = workspace_sync::restore_on_start(state).await;
        if restored {
            publish_sandbox_event(
                state,
                "RepositoryRestored",
                "Restored repository from S3 snapshot",
                false,
            )
            .await;
        }
        restored
    } else {
        false
    };

    // 5. Clone repository (fatal on failure) — skipped when a full repo was
    //    restored from S3 above.
    //
    // `workspace_recloned` tracks whether a *fresh* clone actually replaced the
    // workspace (as opposed to a full S3 restore, an EBS/PVC volume that already
    // held the repo — `clone_repository` skips when `.git` exists — or a legacy
    // overlay onto a fresh clone). When true, ephemeral pre-restart state
    // (gitignored deps/build artifacts, out-of-repo files, unpushed commits /
    // uncommitted changes) is gone, and we tell a resuming agent so below.
    let mut workspace_recloned = false;
    if !restored_full_repo {
        {
            *state.init_phase.write().await = InitPhase::CloningRepository;
        }

        if let Some(ref clone_url) = state.config.git_clone_url {
            if !clone_url.is_empty() {
                // A pre-existing `.git` means the repo persisted (EBS/PVC) and
                // `clone_repository` will skip — so nothing was re-cloned/lost.
                let repo_preexisted = std::path::Path::new(&state.config.repo_dir)
                    .join(".git")
                    .exists();
                match git::clone_repository(&state.config).await {
                    Ok(()) => {
                        workspace_recloned = !repo_preexisted;
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

    // 8. Mirror CLAUDE.md → AGENTS.md throughout the repo so Codex finds the same
    // instructions Claude does (Codex reads the AGENTS.md in the directory of
    // files it edits). Runs unconditionally — agent type is decided per-message;
    // symlinks are cheap, idempotent, and harmless when Claude is the active
    // agent. Skills are handled at the workspace level below, not mirrored here.
    if !state.config.repo_dir.is_empty() {
        let agents_md = codex_compat::mirror_claude_md_to_agents_md(std::path::Path::new(
            &state.config.repo_dir,
        ));
        if agents_md > 0 {
            publish_sandbox_event(
                state,
                "CodexCompatMirrored",
                &format!("Mirrored {agents_md} AGENTS.md for Codex"),
                false,
            )
            .await;
        }
    }

    // 8b. Bridge the repo's root agent config up to the workspace root: skills for
    // both agents (`.claude/skills` for Claude, `.agents/skills` for Codex) plus
    // `CLAUDE.md`/`AGENTS.md`. The agent's cwd is `workspace_dir`, but these are
    // discovered relative to cwd, so a repo checked out one level down at
    // `repo_dir` would otherwise be invisible. Symlinks only — the repo stays the
    // source of truth; they sit outside `repo_dir` so the S3 snapshot skips them,
    // and they're regenerated every init. No-op when there is no separate checkout
    // (`repo_dir == workspace_dir`).
    if !state.config.repo_dir.is_empty() {
        let bridged = workspace_bridge::bridge_repo_config_to_workspace(
            std::path::Path::new(&state.config.workspace_dir),
            std::path::Path::new(&state.config.repo_dir),
        );
        if bridged > 0 {
            info!(
                count = bridged,
                "Bridged repo agent config (skills/CLAUDE.md/AGENTS.md) to workspace root"
            );
        }
    }

    // 9. Restore CLI session state from S3 (via backend-proxied gRPC): Claude
    //    transcripts into `projects/<dashed-cwd>/` and Codex rollout sessions
    //    into `$CODEX_HOME/sessions`. Runs here — after the clone, before the
    //    sandbox is marked Ready — so `recover_interrupted_run` and
    //    pending-message dispatch see restored `--resume` history.
    //    Tier-agnostic and best-effort: a missing archive (or a disabled
    //    feature) is non-fatal.
    transcript_sync::download_on_start(state).await;

    // 10. Legacy S3 overlay: only when workspace sync is on AND we did *not*
    //     already restore a full repo in step 4 (i.e. the stored snapshot is a
    //     working-tree-only "legacy" archive without `.git`, or none existed and
    //     we cloned). Overlays the persisted working tree onto the fresh clone so
    //     the agent sees restored edits. New-format snapshots skip this — they
    //     were fully restored before the clone. Best-effort and non-fatal.
    if state.config.workspace_sync_enabled && !restored_full_repo {
        workspace_sync::download_on_start(state).await;
    }

    // Report how the workspace was reconstituted so a resuming agent gets the
    // right restart note. A full S3 restore keeps the git tree but drops
    // gitignored build artifacts; a fresh clone drops all ephemeral state; a
    // pre-existing volume (or fresh task) lost nothing.
    let origin = if restored_full_repo {
        WorkspaceOrigin::RestoredFromSnapshot
    } else if workspace_recloned {
        WorkspaceOrigin::Recloned
    } else {
        WorkspaceOrigin::Persisted
    };
    Ok(origin)
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

#[cfg(test)]
mod tests {
    use super::*;

    const WS: &str = "/opt/moatless/workspace/repo";

    #[test]
    fn persisted_origin_emits_no_restart_note() {
        assert_eq!(WorkspaceOrigin::Persisted.restart_note(WS), None);
    }

    #[test]
    fn snapshot_restore_warns_about_wiped_build_artifacts_only() {
        let note = WorkspaceOrigin::RestoredFromSnapshot
            .restart_note(WS)
            .expect("snapshot restore must warn the agent");
        assert!(note.contains("<sandbox-restart-note>"));
        assert!(note.contains(WS));
        // Must tell the agent the git tree survived but build artifacts did not.
        assert!(note.contains("restored from"));
        assert!(note.contains("target"));
        assert!(note.contains("evicted"));
        // Must NOT scare the agent about losing uncommitted work — the S3
        // snapshot preserves the non-gitignored working tree.
        assert!(!note.contains("never pushed may also be missing"));
    }

    #[test]
    fn reclone_warns_about_full_ephemeral_loss() {
        let note = WorkspaceOrigin::Recloned
            .restart_note(WS)
            .expect("a fresh reclone must warn the agent");
        assert!(note.contains("<sandbox-restart-note>"));
        assert!(note.contains("fresh `git clone`"));
        // A reclone can lose unpushed commits — the snapshot path cannot.
        assert!(note.contains("never pushed may also be missing"));
    }
}
