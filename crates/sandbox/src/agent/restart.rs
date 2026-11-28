use std::collections::BTreeMap;

use crate::agent::wake_note::{self, WakeNote};
use crate::init::InitPhase;
use crate::state::{SandboxState, TaskContext};
use tracing::{info, warn};

/// Opens the pod for work: decides interrupted-run precedence for each Member
/// while dispatch is gated, flips `InitPhase::Ready`, then dispatches each once.
///
/// Precedence goes first, or a racing notification could ack a newer Message
/// before the probe runs, leaving a stale Recover enqueued behind it.
pub(crate) async fn open_for_work(state: &SandboxState, workspace_origin: WorkspaceOrigin) {
    // Read and clear what a prior pod left behind before a recovered run or a
    // new command can overwrite it.
    let lost_commands =
        take_lost_commands(state, &state.config.task_id, &state.config.workspace_dir).await;
    let wake_notes = wake_note::take(&wake_note::default_path(&state.config.workspace_dir)).await;
    // Before the Ready flip: this container's own `pod_ready` ends the search.
    let stop_cause = look_up_stop_cause(state, workspace_origin).await;

    // Init has placed the tree, so an upload now stores the task's own.
    state.workspace_sync.mark_placed();
    // Adopt whatever the restore or clone produced as the upload baseline,
    // unless a file was consumed: the take deleted it here but S3 still
    // carries it, so adopting now would let an untouched turn skip the
    // upload, and the next replacement would restore the stale archive.
    if state.config.workspace_sync_enabled && lost_commands.is_empty() && wake_notes.is_empty() {
        state.workspace_sync.adopt_current(&state.config).await;
    }

    {
        // Held across the recovery pass and the Ready flip: `register`
        // takes the same lock, so a racing `JoinTask` lands before the
        // snapshot or blocks until Ready and recovers itself. Dropping
        // early would let a Member land between and recover from neither.
        let _registration = state.task_context_registration.lock().await;
        *state.workspace_origin.write().await = workspace_origin;
        *state.stop_cause.write().await = Some(stop_cause.clone());
        for context in state.task_contexts_snapshot().await {
            recover_interrupted_run_unless_pending(state, &context, workspace_origin).await;
        }
        *state.init_phase.write().await = InitPhase::Ready;
    }
    // No event: the watcher publishes `pod_ready` a moment later off the same
    // health endpoint this flips, and that is the row that says the Sandbox is up.
    info!(task_id = %state.config.task_id, "Core init complete — health is now ready");

    // Awaited, not detached, so this stays the one authoritative startup
    // dispatch. Per Member, never the task-less lookup: that resolves only a
    // sole Member, so a shared pod would deliver nothing.
    for context in state.task_contexts_snapshot().await {
        crate::agent::dispatcher::dispatch_pending_messages_for_task(
            state.clone(),
            context.task_id.clone(),
        )
        .await;
    }

    // Detached: a wake waits on the dispatch lock that the turn above holds
    // until the harness accepts it, and the harness waits for the workspace
    // setup that runs after this returns. Awaiting it deadlocks init.
    let state = state.clone();
    crate::agent::dispatcher::spawn_supervised("startup-wake-notes", async move {
        report_wake_notes(&state, wake_notes, lost_commands, &stop_cause).await;
    });
}

/// Reads and clears both copies of what a prior pod left unreported for one
/// Member, merging them so each command is announced once.
///
/// Call before the Member's registry runs a command: each mirror write is
/// absolute, so the first `run` erases both copies this reads.
pub(crate) async fn take_lost_commands(
    state: &SandboxState,
    task_id: &str,
    workspace_dir: &str,
) -> Vec<crate::commands::JournaledCommand> {
    let journal_path = crate::commands::default_journal_path(workspace_dir, task_id);
    let journal = crate::commands::take_journal(&journal_path).await;

    let stored = match state.backend_client.take_unreported_commands(task_id).await {
        Ok(stored) => stored,
        Err(error) => {
            warn!(task_id, %error, "Failed to read the stored unreported commands");
            Vec::new()
        }
    };
    merge_lost_commands(journal, stored)
}

/// Unions the two copies by command id. The journal rides the workspace
/// snapshot and the Sandbox row does not, so either alone may be partial.
///
/// `journal` wins a collision: its write is under the entries lock, so it
/// cannot be the staler of the two.
fn merge_lost_commands(
    mut journal: Vec<crate::commands::JournaledCommand>,
    stored: Vec<crate::commands::JournaledCommand>,
) -> Vec<crate::commands::JournaledCommand> {
    let known: std::collections::HashSet<String> = journal.iter().map(|c| c.id.clone()).collect();
    journal.extend(stored.into_iter().filter(|c| !known.contains(&c.id)));
    journal
}

/// Wakes each Member the replaced pod owes something to: a `moat cmd` command
/// whose watch died with the pod, or a caller (`moat tasks restart --yes`) that
/// died before it could report.
///
/// Delivered through `dispatcher::queue_and_dispatch`, so it lands as a
/// resumable `user` row.
async fn report_wake_notes(
    state: &SandboxState,
    notes: BTreeMap<String, WakeNote>,
    lost: Vec<crate::commands::JournaledCommand>,
    stop_cause: &str,
) {
    for (task_id, text) in wake_messages(&state.config.task_id, notes, &lost, stop_cause) {
        // No idempotency key: each Member is owed one wake, and its text is what
        // distinguishes it.
        crate::agent::dispatcher::queue_and_dispatch(state, &task_id, text, None).await;
    }
}

/// One message per Member owed a wake, wrapped in one `<sandbox-restart-note>`.
/// Lost-command text joins `own_task_id`'s note so a self-restart that also
/// lost commands opens one turn, not two, and leads with `stop_cause`.
fn wake_messages(
    own_task_id: &str,
    notes: BTreeMap<String, WakeNote>,
    lost: &[crate::commands::JournaledCommand],
    stop_cause: &str,
) -> Vec<(String, String)> {
    let mut bodies: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (task_id, note) in notes {
        bodies.entry(task_id).or_default().push(note.text);
    }
    if let Some(body) = lost_commands_body(lost) {
        bodies
            .entry(own_task_id.to_string())
            .or_default()
            .push(format!("{stop_cause}\n{body}"));
    }
    bodies
        .into_iter()
        .map(|(task_id, parts)| (task_id, wrap_restart_note(&parts)))
        .collect()
}

/// The wake a Member joining a replacement pod is owed for commands it lost, or
/// `None` when it is owed none. [`open_for_work`] covers only the Member the
/// pod booted with; a joining Member's journal and row are read here.
pub(crate) fn lost_commands_wake(lost: &[crate::commands::JournaledCommand]) -> Option<String> {
    lost_commands_body(lost).map(|body| wrap_restart_note(std::slice::from_ref(&body)))
}

/// One `<sandbox-restart-note>` around every part a Member is owed, so a Member
/// never gets two.
fn wrap_restart_note(parts: &[String]) -> String {
    format!(
        "<sandbox-restart-note>\n{}\n</sandbox-restart-note>",
        parts.join("\n\n")
    )
}

/// What a restart destroyed, or `None` when no command was owed a wake. A
/// `Running` command was cut off mid-flight; a terminal one finished but was
/// never read. No log file survives, so each command's last lines are quoted
/// from the journal instead.
fn lost_commands_body(lost: &[crate::commands::JournaledCommand]) -> Option<String> {
    use crate::commands::CommandState;
    if lost.is_empty() {
        return None;
    }
    let lines: Vec<String> = lost
        .iter()
        .map(|c| {
            let status = match c.state {
                CommandState::Queued => {
                    "was still queued when the sandbox restarted and never started".to_string()
                }
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
            let head = format!("- `{}` ({}): {status}", c.id, c.label);
            // Indented under its own bullet so several lost commands stay
            // readable, and fenced so the agent does not read a log line that
            // looks like an instruction as one.
            match c.log_tail.as_deref() {
                Some(tail) => {
                    let quoted = tail
                        .lines()
                        .map(|line| format!("  {line}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("{head}\n  Its log ended:\n  ```\n{quoted}\n  ```")
                }
                None => head,
            }
        })
        .collect();
    let noun = if lost.len() == 1 {
        "command was"
    } else {
        "commands were"
    };
    let any_tail = lost.iter().any(|c| c.log_tail.is_some());
    let closing = if any_tail {
        "The last lines of each log are quoted above — they are all that survived, \
         and they are a record of where the command got to, not a result. Judge \
         from them whether a command got far enough to trust, re-run the ones that \
         did not, then continue the task."
    } else {
        "Re-run any of these you still need, then continue the task."
    };
    Some(format!(
        "The sandbox was restarted and {} background {noun} lost with it. Each was \
         started with `moat cmd`; its process and its log file did not survive the \
         restart:\n{}\n\n{closing}",
        lost.len(),
        lines.join("\n")
    ))
}

/// How the workspace came to exist on this start — determines what (if
/// anything) a resuming agent must be warned lost across the restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceOrigin {
    /// Nothing was restored or cloned: a task with no checkout, or a volume
    /// kept without workspace sync. The default; a fresh task resumes no run,
    /// so the kept-volume note it shares with [`Self::KeptVolume`] never shows.
    #[default]
    Persisted,
    /// The restore found the previous container's tree on the volume and kept
    /// it: a PVC, or an `emptyDir` across an in-place container restart.
    KeptVolume,
    /// Restored from the snapshot the previous pod's shutdown flush wrote with
    /// every workspace writer already stopped: each tracked or untracked file
    /// survived, and the gitignored ones were never snapshotted.
    RestoredFromFinalFlush,
    /// Restored from an S3 snapshot: the tree is whatever the last snapshot
    /// that committed held, so a write that landed after it started may be
    /// missing, and gitignored build artifacts were never in it at all.
    RestoredFromSnapshot {
        /// When that snapshot started — the tree's age, which the note quotes so
        /// the agent knows which of its own writes to re-check, and not a bound
        /// on what reached it. `None` when the restore could not date it.
        taken_at: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// Rebuilt from a fresh `git clone` — all ephemeral pre-restart state
    /// (incl. unpushed commits and uncommitted changes) may be gone.
    Recloned,
}

impl WorkspaceOrigin {
    /// The `<sandbox-restart-note>` to prepend to a resumed turn, led by
    /// `stop_cause` when the start looked one up.
    fn restart_note(self, workspace_dir: &str, stop_cause: Option<&str>) -> String {
        let body = match self {
            Self::Persisted | Self::KeptVolume => workspace_kept_note(workspace_dir),
            Self::RestoredFromFinalFlush => workspace_intact_note(workspace_dir),
            Self::RestoredFromSnapshot { taken_at } => {
                workspace_restored_note(workspace_dir, taken_at)
            }
            Self::Recloned => workspace_recloned_note(workspace_dir),
        };
        let cause = stop_cause.map_or_default(|cause| format!("{cause}\n"));
        format!(
            "<sandbox-restart-note>\n{cause}{CUT_OFF_TOOL_CALL_NOTE}\n{body}\n</sandbox-restart-note>\n\n"
        )
    }
}

/// Names the lines Claude Code writes when the shutdown cancels a turn: the
/// sandbox sends the same `interrupt` as the Stop button, so the session blames
/// the user and says to wait. Without this the resumed agent stops and waits.
const CUT_OFF_TOOL_CALL_NOTE: &str = "If your previous turn ends in \
    \"[Request interrupted by user for tool use]\", or in a tool result saying \
    the user doesn't want to proceed and to stop and wait, the sandbox's \
    shutdown wrote it, not the user: nobody interrupted you, and nobody is \
    waiting to tell you how to proceed. That tool call may have run in part or \
    in full, so check what it did before running it again.";

/// Attempts at [`look_up_stop_cause`] for a kept volume, and the wait between.
/// The watcher writes an in-place kill's event on its own schedule, and a kept
/// volume skips the restore that otherwise gives it time to land.
const STOP_CAUSE_ATTEMPTS: u32 = 4;
const STOP_CAUSE_RETRY: std::time::Duration = std::time::Duration::from_secs(2);

/// Why the previous container stopped, read before this one reports Ready.
/// A failed read is worded as unknown: the note still owes the rest.
async fn look_up_stop_cause(state: &SandboxState, origin: WorkspaceOrigin) -> String {
    let attempts = if origin == WorkspaceOrigin::KeptVolume {
        STOP_CAUSE_ATTEMPTS
    } else {
        1
    };
    let task_id = &state.config.task_id;
    let cause = await_stop_cause(attempts, STOP_CAUSE_RETRY, || {
        state.backend_client.get_last_stop_cause(task_id)
    })
    .await;
    describe_stop_cause(cause.as_ref())
}

/// Calls `fetch` until it names a cause, at most `attempts` times. An error
/// ends the wait: a backend that cannot answer will not answer on a retry.
async fn await_stop_cause<F, Fut>(
    attempts: u32,
    retry: std::time::Duration,
    mut fetch: F,
) -> Option<sandbox_proto::StopCause>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<sandbox_proto::StopCause>, tonic::Status>>,
{
    for attempt in 1..=attempts {
        match fetch().await {
            Ok(Some(cause)) => return Some(cause),
            Ok(None) if attempt < attempts => tokio::time::sleep(retry).await,
            Ok(None) => {}
            Err(error) => {
                warn!(%error, "Failed to read why the previous container stopped");
                return None;
            }
        }
    }
    None
}

/// The restart note's first line: what stopped the previous container, and
/// whether load in this sandbox could have caused it.
fn describe_stop_cause(cause: Option<&sandbox_proto::StopCause>) -> String {
    let Some(cause) = cause else {
        return "Moatless has no record of why the previous container stopped; it may \
                not have been recorded yet, so do not read this as a clean restart."
            .to_string();
    };
    let at = chrono::DateTime::from_timestamp_millis(cause.occurred_at_unix_ms)
        .map_or_default(|at| format!(" at {}", at.format("%H:%M:%S UTC")));
    let mut facts = Vec::new();
    if let Some(code) = cause.exit_code {
        facts.push(format!("exit {code}"));
    }
    if let Some(reason) = cause.terminated_reason.as_deref() {
        facts.push(reason.to_string());
    }
    let facts = if facts.is_empty() {
        String::new()
    } else {
        format!(" ({})", facts.join(", "))
    };
    let oom = cause.kind == "container_oom_killed"
        || cause.terminated_reason.as_deref() == Some("OOMKilled");
    match cause.kind.as_str() {
        _ if oom => format!(
            "The previous container was OOMKilled{at}{facts}: the processes in this \
             sandbox together used more than its memory limit. Run fewer memory-heavy \
             processes at once (builds, test runners, browsers) before you retry."
        ),
        "node_reclaimed" | "pod_evicted" | "pod_failed" | "pod_deleted" => format!(
            "The previous pod was removed{at} ({}){facts}. If that reason names this \
             sandbox's own disk or memory use, lower it before you retry; otherwise \
             nothing in this sandbox caused the restart.",
            cause.disruption_reason.as_deref().unwrap_or(&cause.kind)
        ),
        "stopped" => format!(
            "The sandbox was stopped on purpose{at}, by {}.",
            cause.initiator.as_deref().unwrap_or("an unknown initiator")
        ),
        _ if cause.exit_code == Some(137) => format!(
            "Kubernetes killed the previous container{at}{facts} without an OOMKilled \
             reason. That is most often a failed liveness probe on a node short of \
             memory or CPU, which any sandbox on the node can cause; retry, and lower \
             this sandbox's load if it happens again."
        ),
        _ if cause.exit_code == Some(0) => format!(
            "The previous container was told to stop and exited cleanly{at}{facts}, most \
             often after a failed liveness probe on a node short of memory or CPU."
        ),
        _ => format!("The previous container stopped{at}{facts}: {}.", cause.kind),
    }
}

/// Note prepended to a resumed turn when the volume was kept, so the files
/// are as the previous container left them but none of its processes are.
fn workspace_kept_note(workspace_dir: &str) -> String {
    format!(
        "This sandbox was restarted, and the workspace at {workspace_dir} was kept \
         on its volume as the previous container left it, gitignored files such as \
         node_modules and target included. Every process it ran is gone, `moat cmd` \
         commands and servers among them, and /tmp was cleared: restart what you \
         still need, then continue."
    )
}

/// Note prepended to a resumed turn when the shutdown flush wrote the restored
/// snapshot.
///
/// The claim reaches exactly the snapshotted file set and no further, so it
/// exempts every gitignored path rather than the build artifacts alone.
fn workspace_intact_note(workspace_dir: &str) -> String {
    format!(
        "This sandbox was restarted, and the workspace at {workspace_dir} was \
         restored from the snapshot the previous pod took once the agent and \
         every background command had stopped. Every file git tracks or would \
         report as untracked came back: continue without auditing the tree.\n\
         Gitignored paths were never snapshotted and are gone — build artifacts \
         and dependencies (target, node_modules, dist, .venv), and anything else \
         your ignore rules cover, such as a .env you wrote. Re-run install/build \
         steps and re-create any ignored file you rely on."
    )
}

/// Note prepended to a resumed turn when the workspace was restored from an S3
/// snapshot (pod replaced, reclaimed with the node, or evicted).
///
/// Dates the tree and promises nothing: `snapshot` lists paths before
/// `to_snapshot` stamps the time, so no instant bounds what reached one. Hence
/// the stamp, not `summary.backup_end`: a stronger-sounding promise, no truer.
fn workspace_restored_note(
    workspace_dir: &str,
    taken_at: Option<chrono::DateTime<chrono::Utc>>,
) -> String {
    let age = match taken_at {
        Some(at) => format!(
            "That snapshot began at {}, which dates the tree rather than \
             guaranteeing it: edits, local commits and new files from around or \
             after then may be missing.",
            at.format("%Y-%m-%d %H:%M:%S UTC")
        ),
        None => "The restore could not date that snapshot, so work from your \
                 last turns may not have reached it."
            .to_string(),
    };
    format!(
        "This sandbox was restarted, and the workspace at {workspace_dir} was \
         restored from its last completed snapshot. {age} Run `git status` and \
         `git diff` in each repo against what you believe you wrote, and redo \
         what is missing rather than assuming it is there.\n\
         Gitignored paths are never snapshotted and are gone regardless — build \
         artifacts and dependencies (target, node_modules, dist, .venv), and \
         anything else your ignore rules cover, such as a .env you wrote. Re-run \
         install/build steps and re-create any ignored file you rely on. \
         Everything else under {workspace_dir} is snapshotted, including repos \
         beside the primary one and loose files, so keep whatever must survive \
         the next restart there and not in `/tmp`."
    )
}

/// Note prepended to a resumed turn when the workspace was rebuilt from a fresh
/// clone across the restart, so the agent knows its pre-restart scratch state
/// is gone rather than silently working against a reset tree.
fn workspace_recloned_note(workspace_dir: &str) -> String {
    format!(
        "This sandbox was restarted (the pod was replaced or evicted) and the \
         repository at {workspace_dir} was rebuilt from a fresh `git clone`. \
         Committed, pushed git content is present, but ephemeral state from \
         before the restart is gone: gitignored dependencies and build artifacts \
         (e.g. node_modules, target, dist, .venv), and any files created outside \
         the git repository. Any local commits or uncommitted changes that were \
         never pushed may also be missing. Re-run install/build steps and verify \
         your working state before continuing."
    )
}
/// Interrupted-run precedence gate for one Member: replays the interrupted
/// run only when no pending user message is waiting. Keys on `context`,
/// never `state.config`.
///
/// A probe error fails closed. Call once per Member per start: covered by
/// [`open_for_work`] at init, [`crate::task_context::register`] after Ready.
pub(crate) async fn recover_interrupted_run_unless_pending(
    state: &SandboxState,
    context: &TaskContext,
    origin: WorkspaceOrigin,
) {
    let task_id = &context.task_id;
    // Fail closed: an unknown probe result is treated as "pending present".
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
        note_superseded_run(state, context, origin).await;
    } else {
        recover_interrupted_run(state, context, origin).await;
    }
}

/// Hold a restart note for the turn the pending messages open, when they
/// supersede an interrupted run. Without it the resumed session ends on a cut-off
/// tool call, which the agent reads as the user interrupting it.
async fn note_superseded_run(state: &SandboxState, context: &TaskContext, origin: WorkspaceOrigin) {
    let task_id = &context.task_id;
    match state.backend_client.probe_interrupted_run(task_id).await {
        Ok(Some(turn)) => {
            info!(
                task_id = %task_id,
                turn_number = turn.turn_number,
                "Pending messages supersede the interrupted run; noting it on their turn"
            );
            let stop_cause = state.stop_cause.read().await.clone();
            let mut note = superseded_run_note(turn.turn_number);
            note.push_str(&origin.restart_note(&context.checkout_dir, stop_cause.as_deref()));
            context.set_restart_note(note);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, task_id = %task_id, "Failed to probe for a superseded interrupted run");
        }
    }
}

/// Note leading the turn that runs pending messages in place of replaying the
/// interrupted turn `turn_number`.
fn superseded_run_note(turn_number: i64) -> String {
    format!(
        "<sandbox-restart-note>\n\
         This sandbox was restarted in the middle of turn {turn_number}, and the \
         messages below arrived before it could resume, so they run instead of a \
         replay. Finish what it left undone unless the messages below change \
         the plan.\n\
         </sandbox-restart-note>\n\n"
    )
}

/// Detects `context`'s interrupted agent run via the backend RPC and queues a
/// resume through the existing `--resume <session_id>` path; a no-op if there
/// is none.
async fn recover_interrupted_run(
    state: &SandboxState,
    context: &TaskContext,
    origin: WorkspaceOrigin,
) {
    let task_id = &context.task_id;
    match state.backend_client.recover_interrupted_run(task_id).await {
        Ok(Some(turn)) => {
            tracing::warn!(
                task_id = %task_id,
                session_id = %turn.session_id,
                turn_number = turn.turn_number,
                workspace_origin = ?origin,
                "Resuming interrupted agent run"
            );
            let stop_cause = state.stop_cause.read().await.clone();
            let note = origin.restart_note(&context.checkout_dir, stop_cause.as_deref());
            let prompt_text = format!("{note}{}", turn.prompt_text);
            context.agent_handle.recover(
                prompt_text,
                turn.agent_mode,
                turn.effort,
                turn.model,
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
    const CAUSE: &str = "Kubernetes killed the previous container.";

    fn lost(id: &str, state: CommandState) -> JournaledCommand {
        JournaledCommand {
            id: id.to_string(),
            command: format!("cargo test {id}"),
            label: id.to_string(),
            started_at_unix_ms: 1,
            state,
            exit_code: None,
            log_tail: None,
        }
    }

    #[test]
    fn merge_unions_both_copies_and_lets_the_journal_win() {
        let merged = merge_lost_commands(
            vec![lost("a", CommandState::Exited)],
            vec![
                lost("a", CommandState::Running),
                lost("b", CommandState::Running),
            ],
        );
        assert_eq!(
            merged,
            vec![
                lost("a", CommandState::Exited),
                lost("b", CommandState::Running)
            ],
            "the row contributes what the journal never saw, and never overwrites it"
        );
    }

    #[test]
    fn merge_carries_the_row_alone_when_the_snapshot_was_lost() {
        // The failure this whole path exists for: a restore that produced no
        // journal still names what the restart destroyed.
        let merged = merge_lost_commands(vec![], vec![lost("a", CommandState::Running)]);
        assert_eq!(merged, vec![lost("a", CommandState::Running)]);
    }

    #[test]
    fn a_lost_command_quotes_the_log_tail_the_journal_carried() {
        let mut running = lost("a", CommandState::Running);
        running.log_tail = Some("PKG @t3tools/web PASS 132s\nPKG @t3tools/desktop FAIL(1)".into());

        let body = lost_commands_body(&[running]).expect("a lost command owes a wake");

        assert!(body.contains("PKG @t3tools/web PASS 132s"));
        assert!(body.contains("PKG @t3tools/desktop FAIL(1)"));
        // The whole point: an agent must be able to judge how far the command
        // got, rather than being told only that the result is unknown.
        assert!(body.contains("Its log ended:"));
        assert!(
            !body.contains("there is nothing left to read back"),
            "the closing line must not contradict a quoted tail: {body}"
        );
    }

    #[test]
    fn a_lost_command_without_a_tail_still_reads_as_it_did() {
        // An older pod's journal has no tail, and a command killed before it
        // wrote a line has nothing to quote. Neither may grow an empty block.
        let body = lost_commands_body(&[lost("a", CommandState::Running)])
            .expect("a lost command owes a wake");

        assert!(!body.contains("Its log ended:"));
        assert!(body.contains("Re-run any of these you still need"));
    }

    /// A kept volume lost no file, but every process: an agent told only
    /// that the tree is intact would wait on a server that no longer runs.
    #[test]
    fn a_kept_volume_says_the_files_survived_and_the_processes_did_not() {
        let note = WorkspaceOrigin::KeptVolume.restart_note(WS, Some(CAUSE));

        assert!(
            note.starts_with(&format!("<sandbox-restart-note>\n{CAUSE}\n")),
            "{note}"
        );
        assert!(note.contains("node_modules"), "{note}");
        assert!(note.contains("Every process it ran is gone"), "{note}");
        assert!(!note.contains("git status"), "{note}");
    }

    /// The incident: an in-place kill with no `OOMKilled` reason read, to the
    /// agent, like its own memory use. The note must say what it most likely was.
    #[test]
    fn each_recorded_cause_is_named_in_the_note() {
        let cause = |kind: &str, exit_code: Option<i32>, reason: Option<&str>| {
            describe_stop_cause(Some(&sandbox_proto::StopCause {
                kind: kind.to_string(),
                terminated_reason: reason.map(str::to_string),
                exit_code,
                disruption_reason: Some("DeletionByPodGC".to_string()),
                initiator: Some("reaper".to_string()),
                occurred_at_unix_ms: 1_757_000_000_000,
            }))
        };

        let oom = cause("container_oom_killed", Some(137), Some("OOMKilled"));
        assert!(oom.contains("OOMKilled at 15:33:20 UTC (exit 137"), "{oom}");
        let liveness = cause("container_restarted", Some(137), Some("Error"));
        assert!(liveness.contains("liveness probe"), "{liveness}");
        assert!(!liveness.contains("OOMKilled at"), "{liveness}");
        let node = cause("node_reclaimed", None, None);
        assert!(node.contains("(DeletionByPodGC)"), "{node}");
        let stopped = cause("stopped", None, None);
        assert!(stopped.contains("by reaper"), "{stopped}");
        assert!(describe_stop_cause(None).contains("not have been recorded yet"));
    }

    /// The reviewer's race: the kill's event lands after a kept volume's fast
    /// init first asks, so a single read would call it unrecorded.
    #[tokio::test(start_paused = true)]
    async fn a_cause_recorded_after_the_first_read_is_still_found() {
        let calls = std::sync::atomic::AtomicU32::new(0);
        let cause = await_stop_cause(STOP_CAUSE_ATTEMPTS, STOP_CAUSE_RETRY, || {
            let call = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                Ok((call == 2).then(|| sandbox_proto::StopCause {
                    kind: "container_restarted".to_string(),
                    ..Default::default()
                }))
            }
        })
        .await;

        assert_eq!(
            cause.map(|c| c.kind).as_deref(),
            Some("container_restarted")
        );
        assert_eq!(calls.into_inner(), 3);
    }

    /// A start with nothing to explain waits for nothing: one read, then on.
    #[tokio::test(start_paused = true)]
    async fn an_unrecorded_cause_is_read_at_most_the_attempts_given() {
        let calls = std::sync::atomic::AtomicU32::new(0);
        let cause = await_stop_cause(1, STOP_CAUSE_RETRY, || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Ok(None) }
        })
        .await;

        assert_eq!(cause, None);
        assert_eq!(calls.into_inner(), 1);
    }

    /// The note is the only thing that tells a resuming agent the tree may be
    /// behind its own last writes, so it must name when the snapshot started
    /// and send the agent to `git status` rather than assert the tree is intact.
    #[test]
    fn a_dated_snapshot_restore_names_its_start_and_sends_the_agent_to_check() {
        let taken_at = chrono::DateTime::from_timestamp(1_757_000_000, 0).expect("a valid instant");
        let note = WorkspaceOrigin::RestoredFromSnapshot {
            taken_at: Some(taken_at),
        }
        .restart_note(WS, None);

        assert!(note.contains("<sandbox-restart-note>"));
        assert!(note.contains(WS));
        assert!(note.contains("restored"));
        // The tree's age, so the agent knows which writes to re-check.
        assert!(note.contains("began at 2025-09-04 15:33:20 UTC"), "{note}");
        // No instant bounds what reached a snapshot, so the note claims none.
        assert!(note.contains("rather than guaranteeing it"), "{note}");
        assert!(
            note.contains("around or after then") && note.contains("may be missing"),
            "{note}"
        );
        assert!(note.contains("git status"), "{note}");
        // Build artifacts are still called out: they are gone on every restore.
        assert!(note.contains("target"));
        // Asserting the tree is intact is what stops an agent checking.
        assert!(!note.contains("intact and present"), "{note}");
    }

    /// An undatable snapshot must read as unknown, never as current — the
    /// rescue tar carries no time and is the older copy by construction.
    #[test]
    fn an_undated_snapshot_restore_says_unknown_rather_than_current() {
        let note = WorkspaceOrigin::RestoredFromSnapshot { taken_at: None }.restart_note(WS, None);

        assert!(note.contains("could not date that snapshot"), "{note}");
        assert!(note.contains("git status"), "{note}");
        assert!(!note.contains("intact and present"), "{note}");
    }

    /// The point of the tag: a flushed tree is reported as whole, so the agent
    /// resumes instead of spending a turn re-checking files nothing touched.
    #[test]
    fn a_final_flush_restore_reports_the_tree_whole_and_asks_for_no_audit() {
        let note = WorkspaceOrigin::RestoredFromFinalFlush.restart_note(WS, None);

        assert!(note.contains("<sandbox-restart-note>"));
        assert!(note.contains(WS));
        assert!(note.contains("Every file git tracks"), "{note}");
        assert!(note.contains("without auditing"), "{note}");
        // The two hedges the dated note carries have no place here, and an
        // agent that reads either still burns the turn this note exists to save.
        assert!(!note.contains("may be missing"), "{note}");
        assert!(!note.contains("git status"), "{note}");
        // The exemption is every ignored path, not the build artifacts alone:
        // an agent that wrote a `.env` gets no all-clear on it.
        assert!(note.contains("target") && note.contains("Re-run"), "{note}");
        assert!(note.contains(".env"), "{note}");
    }

    /// Both restore notes describe the same file set, so neither may name only
    /// the build directories and leave an agent's ignored `.env` unaccounted.
    #[test]
    fn neither_restore_note_limits_the_exemption_to_build_output() {
        for origin in [
            WorkspaceOrigin::RestoredFromFinalFlush,
            WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
        ] {
            let note = origin.restart_note(WS, None);
            assert!(note.contains(".env"), "{origin:?}: {note}");
        }
    }

    /// Regression: a resumed agent read Claude Code's cancel lines as the user
    /// stopping it, and waited for an instruction nobody was going to give.
    #[test]
    fn every_restart_note_disowns_the_cut_off_tool_calls_interruption() {
        for origin in [
            WorkspaceOrigin::Persisted,
            WorkspaceOrigin::KeptVolume,
            WorkspaceOrigin::RestoredFromFinalFlush,
            WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
            WorkspaceOrigin::Recloned,
        ] {
            let note = origin.restart_note(WS, Some(CAUSE));
            assert!(
                note.contains("\"[Request interrupted by user for tool use]\""),
                "{origin:?}: {note}"
            );
            assert!(
                note.contains("nobody interrupted you"),
                "{origin:?}: {note}"
            );
        }
    }

    #[test]
    fn reclone_warns_about_full_ephemeral_loss() {
        let note = WorkspaceOrigin::Recloned.restart_note(WS, None);
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
            log_tail: None,
        }
    }

    #[test]
    fn a_restart_with_no_outstanding_commands_wakes_nobody() {
        assert!(lost_commands_body(&[]).is_none());
    }

    /// The stranding case: the pod was replaced with a command in flight, so no
    /// wake could fire and the agent sat idle until a human prodded it.
    #[test]
    fn a_command_cut_off_mid_flight_is_named_as_unfinished() {
        let body = lost_commands_body(&[journalled(
            "c33e9d6e",
            "verify",
            CommandState::Running,
            None,
        )])
        .expect("an outstanding command must wake the agent");
        assert!(body.contains("c33e9d6e"), "{body}");
        assert!(body.contains("verify"), "{body}");
        assert!(body.contains("did not finish"), "{body}");
        assert!(body.contains("1 background command was"), "{body}");
    }

    /// The outcome outlives the log, so a command that finished unread reports
    /// its exit code rather than sending the agent to a log that is gone.
    #[test]
    fn a_command_that_finished_unread_reports_its_exit_code() {
        let body = lost_commands_body(&[journalled(
            "ab12cd34",
            "build",
            CommandState::Exited,
            Some(101),
        )])
        .expect("an unread outcome is still owed");
        assert!(body.contains("exit 101"), "{body}");
        assert!(!body.contains("did not finish"), "{body}");
    }

    #[test]
    fn several_lost_commands_are_listed_and_counted() {
        let body = lost_commands_body(&[
            journalled("aaaaaaaa", "build", CommandState::Running, None),
            journalled("bbbbbbbb", "test", CommandState::TimedOut, Some(124)),
        ])
        .expect("body");
        assert!(body.contains("2 background commands were"), "{body}");
        assert!(body.contains("aaaaaaaa"), "{body}");
        assert!(body.contains("bbbbbbbb"), "{body}");
        assert!(body.contains("timeout"), "{body}");
    }

    fn noted(text: &str) -> WakeNote {
        WakeNote {
            text: text.to_string(),
            recorded_at_unix_ms: 0,
        }
    }

    /// A restart nobody asked for, with nothing in flight, keeps the pod quiet.
    #[test]
    fn a_restart_that_owes_nothing_sends_nothing() {
        assert!(wake_messages("task-a", BTreeMap::new(), &[], CAUSE).is_empty());
    }

    #[test]
    fn a_wake_note_alone_produces_one_message() {
        let notes = BTreeMap::from([("task-a".to_string(), noted("the restart you asked for"))]);

        let messages = wake_messages("task-a", notes, &[], CAUSE);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].0, "task-a");
        assert!(
            messages[0].1.starts_with("<sandbox-restart-note>"),
            "{}",
            messages[0].1
        );
        assert!(messages[0].1.contains("the restart you asked for"));
    }

    /// One restart opens one turn, so a self-restart that also lost commands
    /// carries both facts in a single message.
    #[test]
    fn a_note_and_lost_commands_travel_in_one_message() {
        let notes = BTreeMap::from([("task-a".to_string(), noted("the restart you asked for"))]);
        let lost = [journalled("aaaaaaaa", "build", CommandState::Running, None)];

        let messages = wake_messages("task-a", notes, &lost, CAUSE);
        assert_eq!(messages.len(), 1);
        let text = &messages[0].1;
        assert!(text.contains("the restart you asked for"), "{text}");
        assert!(text.contains("aaaaaaaa"), "{text}");
        assert_eq!(text.matches("<sandbox-restart-note>").count(), 1, "{text}");
    }

    /// Lost commands belong to the pod's own Member, so a co-member's note must
    /// not absorb them.
    #[test]
    fn lost_commands_go_to_the_pods_own_member() {
        let notes = BTreeMap::from([("task-b".to_string(), noted("task-b restarted"))]);
        let lost = [journalled("aaaaaaaa", "build", CommandState::Running, None)];

        let messages = wake_messages("task-a", notes, &lost, CAUSE);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].0, "task-a");
        assert!(messages[0].1.contains("aaaaaaaa"));
        assert!(
            messages[0].1.contains(CAUSE),
            "the lost commands lead with why"
        );
        assert_eq!(messages[1].0, "task-b");
        assert!(messages[1].1.contains("task-b restarted"));
        assert!(!messages[1].1.contains("aaaaaaaa"));
    }
}
