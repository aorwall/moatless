//! Execute the core and workspace initialization phases.
//!
//! Reserve `publish_sandbox_event` for a step slow enough to wonder about or
//! one that failed; log everything else with `info!`.

use super::{
    InitError, InitPhase, claude_code_settings, codex_config, dirs, git, home_skills,
    language_servers, plugin_skills, workspace,
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
fn publish_sandbox_event(state: &SandboxState, reason: &str, message: &str, is_warning: bool) {
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

    let ready_path = ready_sentinel(&state.config.data_dir);
    clear_ready_sentinel(&ready_path).await;

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
    state.setup_gate.begin_setup();
    run_workspace_setup(&state).await;

    // Write workspace-ready sentinel AFTER setup commands complete (e.g. bun install).
    // Server containers poll for this file before starting, so they must wait
    // until dependencies are installed.
    if let Err(e) = tokio::fs::write(&ready_path, "ready").await {
        warn!(error = %e, "Failed to write workspace-ready sentinel");
    }
    state.setup_gate.open();

    info!(task_id = %state.config.task_id, "Workspace setup complete");
    Ok(())
}

/// The file `moatless_t3code::scripts::SETUP_GATE` and compose servers wait
/// on, written once this boot's setup commands have run.
fn ready_sentinel(data_dir: &str) -> String {
    format!("{data_dir}/.workspace-ready")
}

/// Remove the sentinel an earlier boot left on a kept `DATA_DIR`.
///
/// Left in place, a Script passes the gate before this boot's setup recreates
/// gitignored dependencies, and exits with "not found".
async fn clear_ready_sentinel(path: &str) {
    match tokio::fs::remove_file(path).await {
        Ok(()) => info!("Removed the workspace-ready sentinel an earlier boot left"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => warn!(error = %e, "Failed to remove a stale workspace-ready sentinel"),
    }
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
                .map_or_default(|branch| format!(" ({branch})"));
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

    // 2b. Stage the plugin that gives Claude its `LSP` tool. Non-fatal — the
    //     agent still runs, only without code intelligence.
    let claude_config_dir = PathBuf::from(&state.config.claude_config_dir);
    let servers = state.config.language_servers.clone();
    let runtime_dir = PathBuf::from(&state.config.runtime_dir);
    let search_path = state.config.search_path.clone();
    let staged = run_init_blocking("stage_language_servers", move || {
        language_servers::stage_plugin(
            &claude_config_dir,
            &runtime_dir,
            &servers,
            search_path.as_deref(),
        )
    })
    .await?;
    if let Err(e) = staged {
        warn!(error = %e, "Failed to stage the language server plugin");
    }

    {
        *state.init_phase.write().await = InitPhase::FetchingCredentials;
    }

    if let Err(e) = credentials::fetch_and_apply_credentials(state).await {
        let msg = format!("Failed to fetch credentials: {e}");
        publish_sandbox_event(state, "CredentialsFailed", &msg, true);
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
    );

    // 2. Write the Codex config.toml. Non-fatal — Codex still runs without it.
    if state.config.backend_api_url.is_none() {
        warn!(
            "BACKEND_API_URL is unset, so no agent can reach the model gateway; every model call \
             will fail authentication"
        );
    }
    let gateway_url = state
        .config
        .backend_api_url
        .as_deref()
        .map(codex_config::gateway_base_url);
    let token_file = state.config.persisted_user_token_path();
    let route = gateway_url
        .as_ref()
        .map(|base_url| codex_config::ModelRoute {
            base_url,
            token_file: &token_file,
        });
    codex_config::write_codex_config(&state.config.codex_home_dir, route.as_ref()).await;

    // 2c. Write the Claude Code settings file, after the fetch because the
    //     co-author it credits arrives with the credentials. Safe to overwrite
    //     on every start. Non-fatal — Claude CLI still runs without it.
    let claude_md_excludes = claude_code_settings::claude_md_excludes(&state.config.repo_dirs());
    let credentials = state.credentials.read().await.clone();
    claude_code_settings::write_claude_code_settings(
        &state.config.claude_config_dir,
        &claude_md_excludes,
        crate::agent::co_author::co_author(&credentials),
    )
    .await;

    // 3. Configure git authentication (non-fatal). Writes a config file in
    //    milliseconds, so only the failure is worth saying anything about.
    let git_credentials = state.git_credentials.read().await.clone();
    if let Err(e) = git::configure_git(&state.config, &git_credentials).await {
        warn!(error = %e, "Git configuration failed (non-fatal)");
    }
    git::route_github_through_proxy(&state.config)
        .await
        .map_err(InitError::Permanent)?;

    // 4. For the `s3` backend, try restoring a full-repo snapshot from S3
    //    before cloning: one carrying `.git` is a complete repo at the
    //    prior HEAD/index/working tree, so restoring it skips the clone.
    //    Returns false (workspace left empty, cloned below) for a fresh
    //    task, a disabled feature, a failed transfer, or a tree-only snapshot.
    let restore_decision = if state.config.workspace_sync_enabled {
        let decision = workspace_sync::restore_on_start(state).await;
        if decision.restored_snapshot() {
            publish_sandbox_event(
                state,
                "RepositoryRestored",
                "Restored repository from S3 snapshot",
                false,
            );
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

    // 5. Clone the workspace's repos (fatal on failure); runs even after an
    //    S3 restore, since `clone_repositories` skips a dir that already
    //    has `.git`. `workspace_recloned` is true only for a fresh clone of
    //    the primary repo, not a restore or pre-existing volume — when
    //    true, ephemeral pre-restart state is gone.
    let mut workspace_recloned = false;
    if !state.config.repos.is_empty() {
        {
            *state.init_phase.write().await = InitPhase::CloningRepository;
        }

        match git::clone_repositories(&state.config, &git_credentials).await {
            Ok(summary) => {
                workspace_recloned = summary.primary_recloned && !restored_full_repo;
                if !summary.cloned.is_empty() {
                    publish_sandbox_event(
                        state,
                        "RepositoryCloned",
                        &describe_clone(&state.config, &summary),
                        false,
                    );
                }
            }
            Err(e) => {
                let retryable = e.retryable;
                let msg = format!("Failed to clone repository: {e}");
                publish_sandbox_event(state, "RepositoryCloneFailed", &msg, true);
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

    // After the restore and the clone, so it reaches both a snapshot's
    // `.git/config` and every other checkout under the workspace.
    git::scrub_remote_credentials(&state.config).await;

    // 5. Fetch the task's plugins from the backend (non-fatal). Runs with none
    // picked too, so a restored plugin the owner deselected is removed. The
    // event keeps its `PluginsCloned` reason, which clients match on.
    let fetched = workspace::fetch_plugins(&state.config, state.backend_client.as_ref()).await;
    if !fetched.is_empty() {
        publish_sandbox_event(
            state,
            "PluginsCloned",
            &format!("Fetched {} plugin(s)", fetched.len()),
            false,
        );
    }

    // 6. Stage plugin-bundled skills so the Codex agent can discover them.
    // Runs unconditionally so a plugin fetched above is picked up.
    let plugins_dir = state.config.plugins_dir();
    let staged = run_init_blocking("stage_plugin_skills_for_codex", move || {
        plugin_skills::stage_plugin_skills_for_codex(&plugins_dir)
    })
    .await?;
    if staged > 0 {
        info!(count = staged, "Staged plugin skills for Codex");
    }

    // 8. Install the repo's skills into both agents' home skill dirs
    //    (`<claude_config_dir>/skills`, `<codex_home_dir>/skills`). Skills
    //    are discovered relative to cwd (`workspace_dir`), so a repo at
    //    `repo_dir` would otherwise be invisible; copying them home avoids
    //    pulling the repo's `CLAUDE.md`/`AGENTS.md` into agent context.
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

    // 9. Restore CLI session state from S3: Claude transcripts into
    //    `projects/<dashed-cwd>/`, Codex sessions into `$CODEX_HOME/sessions`.
    //    Runs after the clone, before Ready, so `recover_interrupted_run`
    //    and pending-message dispatch see restored `--resume` history.
    //    Best-effort: a missing archive or disabled feature is non-fatal.
    transcript_sync::download_on_start(state).await;

    // 10. S3 working-tree overlay onto the fresh clone, when sync is on and
    //     step 4 left it owed; best-effort, non-fatal. Suppressed by either
    //     a just-extracted full snapshot (already the working tree) or a
    //     pre-existing durable volume (the archive there is *older* — an
    //     overlay would revert work never captured in a snapshot).
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
    } else if restore_decision == workspace_sync::RestoreDecision::UseExistingVolume {
        WorkspaceOrigin::KeptVolume
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
        );
        if let Err(e) = workspace::run_setup_command(&state.config, cmd).await {
            let msg = format!("Setup command failed: {cmd}: {e}");
            warn!("{msg}");
            publish_sandbox_event(state, "SetupFailed", &msg, true);
            // Non-fatal — continue with remaining setup
        }
    }

    publish_sandbox_event(
        state,
        "WorkspaceSetupComplete",
        "Workspace setup complete",
        false,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The restart that wiped `node_modules` kept `DATA_DIR`: the Script must
    /// wait for this boot's setup, not pass on the last boot's sentinel.
    #[tokio::test]
    async fn a_sentinel_from_an_earlier_boot_is_cleared_before_setup() {
        let data = tempfile::tempdir().expect("tempdir");
        let path = ready_sentinel(&data.path().display().to_string());
        tokio::fs::write(&path, "ready")
            .await
            .expect("stale sentinel");

        clear_ready_sentinel(&path).await;
        assert!(!std::path::Path::new(&path).exists());

        // A first boot has none, which is no failure.
        clear_ready_sentinel(&path).await;
    }

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
