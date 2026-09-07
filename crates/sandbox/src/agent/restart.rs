use crate::init::InitPhase;
use crate::state::SandboxState;
use tracing::{info, warn};

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
            .adopt_current(&state.config, state.backend_client.as_ref())
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
async fn report_lost_commands(state: &SandboxState, lost: Vec<crate::commands::JournaledCommand>) {
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

    fn journalled(
        id: &str,
        label: &str,
        state: CommandState,
        exit_code: Option<i32>,
    ) -> JournaledCommand {
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
