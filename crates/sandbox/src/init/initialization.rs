//! Execute the core and workspace initialization phases.
//!
//! Reserve `publish_sandbox_event` for a step slow enough to wonder about or
//! one that failed; log everything else with `info!`.

use super::{
    InitError, InitPhase, claude_code_settings, codex_config, dirs, git, home_skills,
    plugin_skills, workspace,
};
use crate::agent::restart::{WorkspaceOrigin, open_for_work};
use crate::credentials::fetch as credentials;
use crate::state::SandboxState;
use crate::{transcript_sync, workspace_sync};
use moatless_types::sandbox::SandboxStatus;
use sandbox_proto::PublishEventRequest;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

/// Run one synchronous initialization step on the blocking pool.
///
/// Filesystem helpers recurse and delete trees, which would block a worker if
/// run inline; a `JoinError` becomes `InitError::Transient`, and steps run in
/// the order [`run_core_init`] calls them.
async fn run_init_blocking<T, F>(step: &'static str, work: F) -> Result<T, InitError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work).await.map_err(|e| {
        InitError::Transient(format!(
            "Initialization step '{step}' failed on the blocking pool: {e}"
        ))
    })
}

/// Publish a sandbox init event via gRPC — see module docs for when to use
/// it over `info!`.
///
/// Always `EventType::Sandbox`/`SandboxStatus::Initializing`, with detail in
/// the payload.
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

    let req = PublishEventRequest {
        task_id: state.config.task_id.clone(),
        event_type: sandbox_proto::EventType::Sandbox as i32,
        sub_type: SandboxStatus::Initializing.as_str().to_string(),
        payload_json,
        event_id: String::new(),
        sequence: 0,
        run_id: String::new(),
    };

    state.events.enqueue(req);
}

pub(crate) async fn initialize(state: SandboxState) -> Result<(), InitError> {
    info!(task_id = %state.config.task_id, "Starting sandbox initialization");

    // Phase 1: Core init — must complete before health returns 200.
    // Returns whether the workspace was rebuilt from a fresh clone (ephemeral
    // pre-restart state lost) so we can warn a resuming agent.
    let workspace_origin = run_core_init(&state).await?;

    // Decide interrupted-Run vs pending-Message precedence, open the readiness
    // gate, and perform the one authoritative pending dispatch — in that order.
    // Ordering is the invariant: the precedence probe must run while external
    // dispatch is still gated (see `open_for_work`).
    open_for_work(&state, workspace_origin).await;

    // Phase 2: Workspace setup — runs after health is 200
    run_workspace_setup(&state).await;

    // Server overrides need no step here. They are files under
    // `WORKSPACE_DIR/.moatless/servers/`, so whatever `run_core_init` put on the
    // workspace volume — the PVC as it was left, or the `s3` tier's restored
    // archive — already *is* the current override. There is no second copy to
    // replay and therefore no ordering hazard between the two.

    // Write workspace-ready sentinel AFTER setup commands complete (e.g. bun install).
    // Server containers poll for this file before starting, so they must wait
    // until dependencies are installed.
    let ready_path = format!("{}/.workspace-ready", state.config.data_dir);
    if let Err(e) = tokio::fs::write(&ready_path, "ready").await {
        warn!(error = %e, "Failed to write workspace-ready sentinel");
    }

    info!(task_id = %state.config.task_id, "Workspace setup complete");
    Ok(())
}

/// Human-readable summary of a clone pass, used as the `RepositoryCloned` event
/// message. Names each mount so a multi-repo workspace reports what it got
/// rather than a bare count.
fn describe_clone(config: &crate::config::Config, summary: &git::CloneSummary) -> String {
    let described: Vec<String> = summary
        .cloned
        .iter()
        .map(|mount| {
            let branch = config
                .repos
                .iter()
                .find(|repo| &repo.mount_name == mount)
                .and_then(|repo| repo.branch.as_deref())
                .filter(|branch| !branch.is_empty())
                .map(|branch| format!(" ({branch})"))
                .unwrap_or_default();
            format!("{mount}{branch}")
        })
        .collect();
    match described.len() {
        1 => format!("Cloned repository {}", described[0]),
        n => format!("Cloned {n} repositories: {}", described.join(", ")),
    }
}

/// Phase 1: core init steps that must finish before the pod is marked healthy.
///
/// Returns how the workspace was reconstituted (see [`WorkspaceOrigin`]): a
/// fresh clone loses ephemeral state, an S3 restore loses only gitignored
/// artifacts, and a persisted volume loses nothing.
async fn run_core_init(state: &SandboxState) -> Result<WorkspaceOrigin, InitError> {
    // 1. Ensure directories exist. Wipes `/tmp` and recursively copies the
    //    image-staged skills, so it runs on the blocking pool.
    let dirs_config = Arc::clone(&state.config);
    run_init_blocking("ensure_directories", move || {
        dirs::ensure_directories(&dirs_config);
    })
    .await?;

    // 2. Write the default Claude Code settings file. Static defaults; safe to
    //    overwrite on every start. Non-fatal — Claude CLI still runs without it.
    let claude_md_excludes = claude_code_settings::claude_md_excludes(&state.config.repo_dirs());
    claude_code_settings::write_claude_code_settings(
        &state.config.claude_config_dir,
        &claude_md_excludes,
    )
    .await;

    // 2b. Write the default Codex config.toml. Disables the broken
    //     `codex_apps` MCP server. Non-fatal — Codex still runs without it.
    codex_config::write_codex_config(
        &state.config.codex_home_dir,
        state.config.openai_base_url.as_deref(),
    )
    .await;

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

    // 3. Configure git authentication (non-fatal). Writes a config file in
    //    milliseconds, so only the failure is worth saying anything about.
    if let Err(e) = git::configure_git(&state.config).await {
        warn!(error = %e, "Git configuration failed (non-fatal)");
    }

    // 4. For the `s3` storage backend, try to restore a full-repo snapshot from
    //    S3 *before* cloning. A snapshot that carries `.git` is a complete repo
    //    at the exact prior HEAD + index + dirty working tree, so restoring it
    //    lets us skip the clone entirely. Returns false (leaving the workspace
    //    empty) for a fresh task, a disabled feature, a failed transfer, or a
    //    working-tree-only snapshot — in which case we clone below.
    let restore_decision = if state.config.workspace_sync_enabled {
        let decision = workspace_sync::restore_on_start(state).await;
        if decision.restored_snapshot() {
            publish_sandbox_event(
                state,
                "RepositoryRestored",
                "Restored repository from S3 snapshot",
                false,
            )
            .await;
        }
        decision
    } else {
        workspace_sync::RestoreDecision::Nothing
    };
    let restored_full_repo = restore_decision.restored_snapshot();
    let snapshot_taken_at = restore_decision.snapshot_taken_at();
    let restored_final_flush = restore_decision.restored_final_flush();

    // Ungated and after the restore, so it sees both trees a lock arrives on;
    // `workspace_sync::sweep_git_locks` owns why.
    workspace_sync::sweep_git_locks(PathBuf::from(&state.config.workspace_dir)).await;

    // 5. Clone the workspace's repositories (fatal on failure). Runs even after
    //    an S3 restore: `clone_repositories` skips any repo whose directory
    //    already holds a `.git`, so this only fills in what the snapshot didn't
    //    have — e.g. a repo added to the workspace since it was taken.
    //
    // `workspace_recloned` tracks whether a *fresh* clone actually replaced the
    // primary repo (as opposed to a full S3 restore, an EBS/PVC volume that
    // already held it, or an overlay onto a fresh clone). When true,
    // ephemeral pre-restart state (gitignored deps/build artifacts, out-of-repo
    // files, unpushed commits / uncommitted changes) is gone, and we tell a
    // resuming agent so below.
    let mut workspace_recloned = false;
    if !state.config.repos.is_empty() {
        {
            *state.init_phase.write().await = InitPhase::CloningRepository;
        }

        match git::clone_repositories(&state.config).await {
            Ok(summary) => {
                workspace_recloned = summary.primary_recloned && !restored_full_repo;
                if !summary.cloned.is_empty() {
                    publish_sandbox_event(
                        state,
                        "RepositoryCloned",
                        &describe_clone(&state.config, &summary),
                        false,
                    )
                    .await;
                }
            }
            Err(e) => {
                let retryable = e.retryable;
                let msg = format!("Failed to clone repository: {e}");
                publish_sandbox_event(state, "RepositoryCloneFailed", &msg, true).await;
                // The clone already retried for as long as the failure looked
                // like the network's or the service's. Still failing that way
                // means exit 1, so the restart is the outer retry; anything else
                // is a config error the pod cannot grow out of.
                return Err(if retryable {
                    InitError::Transient(msg)
                } else {
                    InitError::Permanent(msg)
                });
            }
        }
    }

    // 5. Clone the task's plugins (non-fatal). Runs with none picked too, so
    // plugins a restored snapshot still holds are removed.
    let cloned = workspace::clone_plugins(&state.config).await;
    if !cloned.is_empty() {
        publish_sandbox_event(
            state,
            "PluginsCloned",
            &format!("Cloned {} plugin(s)", cloned.len()),
            false,
        )
        .await;
    }

    // 6. Stage plugin-bundled skills so the Codex agent can discover them.
    // Runs unconditionally so a plugin cloned above is picked up.
    let plugins_dir = state.config.plugins_dir();
    let staged = run_init_blocking("stage_plugin_skills_for_codex", move || {
        plugin_skills::stage_plugin_skills_for_codex(&plugins_dir)
    })
    .await?;
    if staged > 0 {
        info!(count = staged, "Staged plugin skills for Codex");
    }

    // 8. Install the repo's skills into the home-level skill dirs for both
    // agents: `<claude_config_dir>/skills` for Claude and `<codex_home_dir>/skills`
    // for Codex. Skills are discovered relative to cwd (`workspace_dir`), so a
    // repo checked out one level down at `repo_dir` would otherwise be invisible;
    // copying into the home dirs (alongside the staged image skills) makes them
    // discoverable without polluting the workspace root or pulling the repo's
    // `CLAUDE.md`/`AGENTS.md` into the agent's project context.
    if !state.config.repo_dir.is_empty() {
        // `install_repo_skills_to_home` walks and copies directory trees, so it
        // runs on the blocking pool like every other filesystem step here.
        let repo_dir = PathBuf::from(&state.config.repo_dir);
        let claude_config_dir = PathBuf::from(&state.config.claude_config_dir);
        let codex_home_dir = PathBuf::from(&state.config.codex_home_dir);
        let installed = run_init_blocking("install_repo_skills_to_home", move || {
            home_skills::install_repo_skills_to_home(&repo_dir, &claude_config_dir, &codex_home_dir)
        })
        .await?;
        if installed > 0 {
            info!(
                count = installed,
                "Installed repo skills into home skill dirs"
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

    // 10. S3 working-tree overlay: only when workspace sync is on AND step 4 left
    //     the overlay owed. Overlays the persisted working tree onto the fresh
    //     clone so the agent sees restored edits. Best-effort and non-fatal.
    //
    //     Both "we just extracted the full snapshot" and "the durable volume
    //     already holds the repo" suppress it: in the first the archive is
    //     already the working tree, and in the second the archive is the *older*
    //     copy — extracting it over a live PVC would revert work that never made
    //     it into a snapshot.
    if state.config.workspace_sync_enabled && !restore_decision.skips_overlay() {
        workspace_sync::download_on_start(state).await;
    }

    // Report how the workspace was reconstituted so a resuming agent gets the
    // right restart note. A full S3 restore returns the tree as of the snapshot
    // it took, which the note dates unless the previous pod's shutdown flush
    // wrote that snapshot and so stored its final tree; a fresh clone drops all
    // ephemeral state; a pre-existing volume (or fresh task) lost nothing.
    let origin = if restored_final_flush {
        WorkspaceOrigin::RestoredFromFinalFlush
    } else if restored_full_repo {
        WorkspaceOrigin::RestoredFromSnapshot {
            taken_at: snapshot_taken_at,
        }
    } else if workspace_recloned {
        WorkspaceOrigin::Recloned
    } else {
        WorkspaceOrigin::Persisted
    };
    Ok(origin)
}

/// Phase 2: run the workspace's configured setup commands.
///
/// Runs after health is ready; each command is non-fatal, only logged and
/// published as a warning event on failure, and says nothing when there are
/// none.
async fn run_workspace_setup(state: &SandboxState) {
    if state.config.setup_commands.is_empty() {
        return;
    }

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
    /// A blocking step that panics must fail initialization, not vanish into
    /// the detached init task. The message has to name the step so the
    /// termination log says which part of setup died.
    #[tokio::test]
    async fn panicking_blocking_step_fails_initialization_with_context() {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = run_init_blocking::<(), _>("ensure_directories", || {
            panic!("injected filesystem failure")
        })
        .await;
        std::panic::set_hook(previous_hook);

        let err = result.expect_err("a panicking blocking step must fail init");
        assert!(
            matches!(err, InitError::Transient(_)),
            "a panicking step is retryable, not a permanent config failure",
        );
        let msg = err.to_string();
        assert!(msg.contains("ensure_directories"), "message was: {msg}");
        assert!(msg.contains("blocking pool"), "message was: {msg}");
    }

    /// The happy path returns the step's value, so counts published as init
    /// events (staged skills, installed repo skills, bridged config) are
    /// unchanged.
    #[tokio::test]
    async fn successful_blocking_step_returns_its_value() {
        let staged = run_init_blocking("stage_plugin_skills_for_codex", || 3usize)
            .await
            .expect("a successful blocking step must not fail init");
        assert_eq!(staged, 3);
    }
}
