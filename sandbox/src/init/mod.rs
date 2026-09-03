//! Sandbox initialization orchestrator.
//!
//! Handles the full lifecycle of workspace setup in two phases:
//!
//! **Phase 1 (core init)**: Credential fetching, git authentication,
//! repository cloning, plugin sync. Health endpoint returns 503 during
//! this phase. On completion, `InitPhase::Ready` is set and health returns 200.
//!
//! **Phase 2 (workspace setup)**: Runs after health is ready. Copies
//! prebuilt deps and runs setup commands.
//! These are non-fatal — failures are logged but don't affect health.

mod claude_code_settings;
mod codex_config;
pub(crate) mod codex_watcher;
pub(crate) mod credentials;
mod dirs;
pub(crate) mod gh_hosts;
pub(crate) mod git;
pub(crate) mod git_push_watcher;
pub(crate) mod home_skills;
pub(crate) mod plugin_skills;
pub(crate) mod refresher;
pub(crate) mod server_env;
mod ssh;
pub(crate) mod transcript_sync;

#[cfg(test)]
mod replacement_test;
mod workspace;
pub mod workspace_sync;

use std::path::PathBuf;
use std::sync::Arc;

use moatless_types::sandbox::SandboxStatus;
use sandbox_proto::PublishEventRequest;
use tracing::{error, info, warn};

use crate::state::SandboxState;
use crate::supervisor::{Criticality, Registrar};

/// Init error with permanence classification.
/// Permanent errors use exit code 78 so the pod watcher stops immediately.
/// Transient errors use exit code 1 and let K8s retry.
#[derive(Debug)]
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

/// Run one synchronous initialization step on the blocking pool.
///
/// The filesystem helpers used by core init recurse over the plugin, skill and
/// repo trees and delete directories; running them inline would park a Tokio
/// worker for the whole traversal. They stay synchronous — and therefore
/// directly unit-testable — and this is the single boundary that bridges them
/// into async initialization.
///
/// A `JoinError` (the step panicked, or the runtime is shutting down) becomes an
/// `InitError` naming the step instead of being logged and dropped. It is
/// classified `Transient` — exit 1, so K8s retries — because nothing about a
/// panicking filesystem step proves the pod configuration can never succeed;
/// only a genuinely unrecoverable config error earns exit 78.
///
/// Steps are awaited one at a time, so initialization ordering is exactly the
/// order the calls appear in [`run_core_init`].
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
/// These land beside the K8s watcher's events as rows a person reads while a
/// thread starts, so a step earns one only when it either takes long enough to
/// wonder about — the credential round-trip, the checkout — or went wrong.
/// Everything else is an `info!`: a row that appears milliseconds after the one
/// above it tells a reader nothing and buries the two that do.
///
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

/// Run the full initialization sequence (phase 1 + phase 2).
///
/// Called from `main.rs` via `tokio::spawn`, under the root supervisor as a
/// finite task: a clean return is the normal end of startup, a panic fails the
/// process. Phase 1 runs first and sets `InitPhase::Ready` on success (or exits
/// on error). Phase 2 runs immediately after Ready, performing workspace setup
/// that doesn't need to block the health endpoint.
///
/// `registrar` is the supervisor's registration handle: the periodic loops
/// started at the end of phase 2 can only be spawned once the workspace exists,
/// so they are registered from here rather than from the composition root.
pub async fn run_init(state: SandboxState, registrar: Registrar) {
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

    // Spawn the periodic credential refresher. GitHub App installation tokens
    // expire after 1h and user access tokens after 8h — long-running tasks
    // would otherwise fail mid-turn on `git push` / `gh pr comment`.
    // See `refresher.rs` for the full rationale.
    //
    // Liveness: the loop handles its own transient errors, so only a panic ends
    // it — after which credentials silently go stale and git operations start
    // failing an hour later with no connection to the cause. A restart re-fetches
    // them; an immediate process exit would kill the turn that is still working.
    let refresher_state = state.clone();
    let refresher_cancel = state.lifecycle_cancel.clone();
    let refresher_interval = refresher::read_interval_from_env();
    registrar.supervise(
        "credential-refresher",
        Criticality::Liveness,
        tokio::spawn(refresher::run_credential_refresher(
            refresher_state,
            refresher_cancel,
            refresher_interval,
        )),
    );

    // Spawn the periodic CLI session-state uploader. Every interval it tars
    // Claude transcripts plus Codex rollout sessions and ships them to S3 via
    // the backend, so agent resume state persists off the per-task PVC (and is
    // the only CLI-session durability for the ephemeral tier).
    // `TRANSCRIPT_SYNC_INTERVAL_SECS=0` disables the loop; sync-on-shutdown
    // still flushes the final delta.
    //
    // Liveness: a dead loop stops persisting resume state, so an ungraceful kill
    // loses everything since the last successful sync — silently, because
    // `flush_on_shutdown` is driven by the shutdown driver rather than by this
    // task and keeps working either way. That final flush is unchanged;
    // supervision only makes the loop's death visible.
    if let Some(interval) = transcript_sync::read_interval_from_env() {
        let transcript_state = state.clone();
        let transcript_cancel = state.lifecycle_cancel.clone();
        registrar.supervise(
            "transcript-sync",
            Criticality::Liveness,
            tokio::spawn(transcript_sync::run_transcript_sync(
                transcript_state,
                transcript_cancel,
                interval,
            )),
        );
    }

    // Spawn the periodic workspace uploader for the `s3` storage backend. Turn
    // completion and the bounded shutdown flush are the primary durability
    // points, but a long turn killed involuntarily (node-pressure eviction, OOM,
    // spot reclaim with no grace) reaches neither and loses everything since the
    // last completed turn. This loop bounds that gap: every interval it
    // fingerprints the workspace and uploads only when the tracked file set
    // changed. `WORKSPACE_SYNC_INTERVAL_SECS=0` disables it, and so does a
    // durable volume, which survives the kill this loop exists for;
    // turn-boundary, pre-stop and shutdown uploads still run.
    //
    // Liveness: a dead loop silently drops the between-turn checkpoints, leaving
    // an ungraceful kill to fall back to the last turn boundary — supervision
    // makes the loop's death visible rather than fixing it.
    if workspace_sync::is_periodic_enabled(&state.config) {
        if let Some(interval) = workspace_sync::read_interval_from_env() {
            let workspace_state = state.clone();
            let workspace_cancel = state.lifecycle_cancel.clone();
            registrar.supervise(
                "workspace-sync",
                Criticality::Liveness,
                tokio::spawn(workspace_sync::run_workspace_sync(
                    workspace_state,
                    workspace_cancel,
                    interval,
                )),
            );
        }
    }

    // Watch ~/.codex/auth.json for rotations. Codex rewrites this file every
    // time it refreshes its OAuth token; the watcher pushes the rotated blob
    // back to the backend so subsequent tasks (and pod restarts) see the new
    // refresh_token instead of the now-invalidated stored copy.
    spawn_codex_auth_watcher(&state, &registrar).await;

    // Watch each checkout's remote reflogs so a sha this task pushes is claimed
    // before GitHub's `pull_request.synchronize` webhook for it arrives.
    spawn_git_push_watcher(&state, &registrar);
}

fn spawn_git_push_watcher(state: &SandboxState, registrar: &Registrar) {
    let watcher_task = git_push_watcher::spawn(
        state.config.task_id.clone(),
        state.config.repo_dirs(),
        std::sync::Arc::clone(&state.backend_client),
        state.lifecycle_cancel.clone(),
    );

    // Optional: losing the watcher costs the task its own push notifications
    // back in its chat. It costs the running turn nothing else, so it is not
    // worth a restart — but it should be named rather than logged as an
    // anonymous panic.
    registrar.supervise(
        "git-push-watcher",
        Criticality::Optional,
        watcher_task,
    );
}

/// Cross the boundary from "restored but private" to "open for work", in the one
/// order that closes the startup precedence race:
///
/// 1. **Decide precedence while dispatch is still gated.** Session state was
///    restored to disk in `run_core_init` (before Ready). We probe for a pending
///    Message and, only when none is waiting, enqueue the interrupted-Run replay.
///    External gRPC dispatch is still a no-op here because init is not yet Ready.
/// 2. **Open the gate.** Flip `InitPhase::Ready` (health now returns 200).
/// 3. **Dispatch once, authoritatively.** Drain any pending Message — a no-op if
///    step 1 already enqueued a Recover, otherwise the Message that superseded it.
///
/// Doing step 1 before step 2 is what prevents a backend notification that races
/// init from fetching, submitting, and acknowledging the newer pending Message in
/// the window before the probe — which would leave the probe seeing no pending
/// row and enqueueing a now-stale Recover behind the Message.
pub(crate) async fn open_for_work(state: &SandboxState, workspace_origin: WorkspaceOrigin) {
    // Read and clear the command journal a prior pod left behind before anything
    // — a recovered run, a new command — can overwrite it. The registry this pod
    // started with is empty, so these entries exist only in the journal.
    let lost_commands = crate::commands::take_journal(&crate::commands::default_journal_path(
        &state.config.workspace_dir,
    ))
    .await;

    recover_interrupted_run_unless_pending(state, workspace_origin).await;

    // Adopt whatever the restore or the clone produced as the upload baseline: a
    // workspace nobody has touched since is not worth shipping back.
    //
    // Consuming a journal is the exception. `take_journal` deleted the file
    // here, but the S3 archive it was restored from still carries it, so this
    // tree is already ahead of the object. Adopting it would let a turn that
    // edits nothing skip the upload, and the next replacement would restore the
    // stale archive and announce the same loss again. Leaving the baseline unset
    // makes the next upload ship the removal.
    if state.config.workspace_sync_enabled && lost_commands.is_empty() {
        state
            .workspace_sync
            .adopt_current(&state.config.workspace_dir)
            .await;
    }

    *state.init_phase.write().await = InitPhase::Ready;
    // No event: the watcher publishes `pod_ready` a moment later off the same
    // health endpoint this flips, and that is the row that says the Sandbox is up.
    info!(task_id = %state.config.task_id, "Core init complete — health is now ready");

    // Awaited, not detached: dispatch only fetches, submits to the agent lifecycle task,
    // and returns (the turn itself runs in the coordinator) — it does not block on
    // the turn, so Phase 2 workspace setup is not delayed. Awaiting keeps this the
    // one authoritative startup dispatch and avoids an orphaned JoinHandle.
    crate::agent::dispatcher::dispatch_pending_messages(state.clone()).await;

    report_lost_commands(state, lost_commands).await;
}

/// Wake the agent for `moat cmd` commands the restart destroyed.
///
/// A `moat cmd` command's process, its completion watch, and its log all live in
/// the pod's memory and `/tmp`, so a pod replacement loses them with no wake —
/// the agent that ended its turn expecting one would sit idle until a human
/// prodded it. The journal on the workspace snapshot is what survives; this
/// turns it into the wake the watch could not deliver. The resumed session
/// (`--continue` for Claude, restored to disk in init) carries the context.
async fn report_lost_commands(
    state: &SandboxState,
    lost: Vec<crate::commands::JournaledCommand>,
) {
    let Some(note) = lost_commands_note(&lost) else {
        return;
    };
    let task_id = &state.config.task_id;
    match state.task_context(Some(task_id)).await {
        Ok(context) => {
            info!(
                task_id = %task_id,
                count = lost.len(),
                "Waking the agent for background commands lost to the restart"
            );
            drop(
                context
                    .agent_handle
                    .send(note, None, None, None, None, None, false, None),
            );
        }
        Err(e) => {
            warn!(error = %e, task_id = %task_id, "Cannot report lost commands without task context");
        }
    }
}

/// The wake for commands a restart destroyed, or `None` when none were owed.
/// A command still `Running` at the snapshot was cut off mid-flight; a terminal
/// one had finished but the agent never read the outcome. Neither log survives,
/// so the note carries what the journal knew and tells the agent to re-run what
/// it still needs.
fn lost_commands_note(lost: &[crate::commands::JournaledCommand]) -> Option<String> {
    use crate::commands::CommandState;
    if lost.is_empty() {
        return None;
    }
    let lines: Vec<String> = lost
        .iter()
        .map(|c| {
            let status = match c.state {
                CommandState::Running => {
                    "was still running when the sandbox restarted and did not finish; its result is unknown".to_string()
                }
                CommandState::Exited => match c.exit_code {
                    Some(0) => "had finished successfully, but you were never shown its output".to_string(),
                    Some(code) => format!("had finished with exit {code}, but you were never shown its output"),
                    None => "had finished, but you were never shown its output".to_string(),
                },
                CommandState::TimedOut => "had hit its timeout and been killed".to_string(),
                CommandState::Killed => "had been killed".to_string(),
            };
            format!("- `{}` ({}): {status}", c.id, c.label)
        })
        .collect();
    let noun = if lost.len() == 1 {
        "command was"
    } else {
        "commands were"
    };
    Some(format!(
        "<sandbox-restart-note>\n\
         The sandbox was restarted and {} background {noun} lost with it. Each was \
         started with `moat cmd`; its process and its log did not survive the \
         restart, so there is nothing left to read back:\n{}\n\n\
         Re-run any of these you still need, then continue the task.\n\
         </sandbox-restart-note>",
        lost.len(),
        lines.join("\n")
    ))
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
/// the agent lifecycle task. Runs once on startup, **only when there are no new pending
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
    let context = match state.task_context(Some(task_id)).await {
        Ok(context) => context,
        Err(e) => {
            tracing::warn!(error = %e, task_id = %task_id, "Cannot recover interrupted run without task context");
            return;
        }
    };
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
            context.agent_handle.recover(
                prompt_text,
                turn.agent_type,
                turn.agent_mode,
                turn.effort,
                turn.session_id,
                turn.run_id,
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

async fn spawn_codex_auth_watcher(state: &SandboxState, registrar: &Registrar) {
    let auth_path = std::path::PathBuf::from(format!("{}/auth.json", state.config.codex_home_dir));

    // Capture the current on-disk value as the baseline so the first rotation
    // (which produces a different blob) triggers the sync. If the file is
    // absent — Codex isn't configured for this task — the watcher still runs
    // and will pick up the file once it appears.
    let initial_value = tokio::fs::read_to_string(&auth_path)
        .await
        .unwrap_or_default();

    let (handle, watcher_task) = codex_watcher::spawn(
        state.config.task_id.clone(),
        auth_path,
        initial_value,
        std::sync::Arc::clone(&state.backend_client),
        state.lifecycle_cancel.clone(),
    );

    // Optional: losing the watcher means a Codex token rotation is not pushed
    // back to the backend, which costs the *next* task (or the next pod start) a
    // stale refresh_token. It costs the running turn nothing, so it is not worth
    // a restart — but it should be named rather than logged as an anonymous
    // panic.
    registrar.supervise("codex-auth-watcher", Criticality::Optional, watcher_task);

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

    // 5. Clone the task's plugins (non-fatal)
    if !state.config.plugins.is_empty() {
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
    }

    // 6. Stage plugin-bundled skills so the Codex agent can discover them.
    // Runs unconditionally so a plugin cloned above is picked up.
    let staged = run_init_blocking(
        "stage_plugin_skills_for_codex",
        plugin_skills::stage_plugin_skills_for_codex,
    )
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

/// Phase 2: Workspace setup — run the workspace's configured setup commands.
///
/// Runs after health is ready. Every command is non-fatal — a failure is logged
/// and published as a warning event but doesn't affect pod health.
///
/// A workspace that configures no setup commands says nothing here: there is no
/// setup for a completion to be the end of.
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
    use crate::commands::{CommandState, JournaledCommand};

    const WS: &str = "/opt/moatless/workspace/moatless";

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

    /// A blocking step that panics must fail initialization, not vanish into the
    /// detached init task. The message has to name the step so the termination
    /// log says which part of setup died.
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
    /// events (staged skills, installed repo skills, bridged config) are unchanged.
    #[tokio::test]
    async fn successful_blocking_step_returns_its_value() {
        let staged = run_init_blocking("stage_plugin_skills_for_codex", || 3usize)
            .await
            .expect("a successful blocking step must not fail init");
        assert_eq!(staged, 3);
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

    fn journalled(id: &str, label: &str, state: CommandState, exit_code: Option<i32>) -> JournaledCommand {
        JournaledCommand {
            id: id.to_string(),
            command: "bash /tmp/verify.sh".to_string(),
            label: label.to_string(),
            started_at_unix_ms: 0,
            state,
            exit_code,
        }
    }

    #[test]
    fn a_restart_with_no_outstanding_commands_wakes_nobody() {
        assert!(lost_commands_note(&[]).is_none());
    }

    /// The stranding case: the pod was replaced with a command in flight, so no
    /// wake could fire and the agent sat idle until a human prodded it.
    #[test]
    fn a_command_cut_off_mid_flight_is_named_as_unfinished() {
        let note = lost_commands_note(&[journalled(
            "c33e9d6e",
            "verify",
            CommandState::Running,
            None,
        )])
        .expect("an outstanding command must wake the agent");
        assert!(note.contains("<sandbox-restart-note>"), "{note}");
        assert!(note.contains("c33e9d6e"), "{note}");
        assert!(note.contains("verify"), "{note}");
        assert!(note.contains("did not finish"), "{note}");
        assert!(note.contains("1 background command was"), "{note}");
    }

    /// The outcome outlives the log, so a command that finished unread reports
    /// its exit code rather than sending the agent to a log that is gone.
    #[test]
    fn a_command_that_finished_unread_reports_its_exit_code() {
        let note = lost_commands_note(&[journalled(
            "ab12cd34",
            "build",
            CommandState::Exited,
            Some(101),
        )])
        .expect("an unread outcome is still owed");
        assert!(note.contains("exit 101"), "{note}");
        assert!(!note.contains("did not finish"), "{note}");
    }

    #[test]
    fn several_lost_commands_are_listed_and_counted() {
        let note = lost_commands_note(&[
            journalled("aaaaaaaa", "build", CommandState::Running, None),
            journalled("bbbbbbbb", "test", CommandState::TimedOut, Some(124)),
        ])
        .expect("note");
        assert!(note.contains("2 background commands were"), "{note}");
        assert!(note.contains("aaaaaaaa"), "{note}");
        assert!(note.contains("bbbbbbbb"), "{note}");
        assert!(note.contains("timeout"), "{note}");
    }
}
