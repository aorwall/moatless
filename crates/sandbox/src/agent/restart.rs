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

    // Adopt whatever the restore or clone produced as the upload baseline — unless
    // either file was consumed: the take deleted it here but the S3 archive still
    // carries it, so adopting now would let an untouched turn skip the upload, and
    // the next replacement would restore the stale archive and repeat the wake.
    if state.config.workspace_sync_enabled && lost_commands.is_empty() && wake_notes.is_empty() {
        state.workspace_sync.adopt_current(&state.config).await;
    }

    {
        // Held across the recovery pass and the Ready flip: `task_context::register`
        // takes the same lock, so a racing `JoinTask` either lands before the
        // snapshot (recovered here) or blocks until Ready and recovers itself.
        // Dropping it early would let a Member land in between and recover from neither.
        let _registration = state.task_context_registration.lock().await;
        *state.workspace_origin.write().await = workspace_origin;
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
        report_wake_notes(&state, wake_notes, lost_commands).await;
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
) {
    for (task_id, text) in wake_messages(&state.config.task_id, notes, &lost) {
        // No idempotency key: each Member is owed one wake, and its text is what
        // distinguishes it.
        crate::agent::dispatcher::queue_and_dispatch(state, &task_id, text, None).await;
    }
}

/// One message per Member owed a wake, wrapped in one `<sandbox-restart-note>`.
/// Lost-command text joins `own_task_id`'s note so a self-restart that also
/// lost commands opens one turn, not two.
fn wake_messages(
    own_task_id: &str,
    notes: BTreeMap<String, WakeNote>,
    lost: &[crate::commands::JournaledCommand],
) -> Vec<(String, String)> {
    let mut bodies: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (task_id, note) in notes {
        bodies.entry(task_id).or_default().push(note.text);
    }
    if let Some(body) = lost_commands_body(lost) {
        bodies
            .entry(own_task_id.to_string())
            .or_default()
            .push(body);
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
    /// Repo persisted in place, or this is a fresh task — nothing was lost. The
    /// default, since most starts need no restart note.
    #[default]
    Persisted,
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
    /// The `<sandbox-restart-note>` to prepend to a resumed turn, or `None`
    /// when nothing was lost and the agent needs no warning.
    fn restart_note(self, workspace_dir: &str) -> Option<String> {
        match self {
            Self::Persisted => None,
            Self::RestoredFromFinalFlush => Some(workspace_intact_note(workspace_dir)),
            Self::RestoredFromSnapshot { taken_at } => {
                Some(workspace_restored_note(workspace_dir, taken_at))
            }
            Self::Recloned => Some(workspace_recloned_note(workspace_dir)),
        }
    }
}

/// Note prepended to a resumed turn when the shutdown flush wrote the restored
/// snapshot.
///
/// The claim reaches exactly the snapshotted file set and no further, so it
/// exempts every gitignored path rather than the build artifacts alone.
fn workspace_intact_note(workspace_dir: &str) -> String {
    format!(
        "<sandbox-restart-note>\n\
         This sandbox was restarted, and the workspace at {workspace_dir} was \
         restored from the snapshot the previous pod took once the agent and \
         every background command had stopped. Every file git tracks or would \
         report as untracked came back: continue without auditing the tree.\n\
         Gitignored paths were never snapshotted and are gone — build artifacts \
         and dependencies (target, node_modules, dist, .venv), and anything else \
         your ignore rules cover, such as a .env you wrote. Re-run install/build \
         steps and re-create any ignored file you rely on.\n\
         </sandbox-restart-note>\n\n"
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
        "<sandbox-restart-note>\n\
         This sandbox was restarted, and the workspace at {workspace_dir} was \
         restored from its last completed snapshot. {age} Run `git status` and \
         `git diff` in each repo against what you believe you wrote, and redo \
         what is missing rather than assuming it is there.\n\
         Gitignored paths are never snapshotted and are gone regardless — build \
         artifacts and dependencies (target, node_modules, dist, .venv), and \
         anything else your ignore rules cover, such as a .env you wrote. Re-run \
         install/build steps and re-create any ignored file you rely on. \
         Everything else under {workspace_dir} is snapshotted, including repos \
         beside the primary one and loose files, so keep whatever must survive \
         the next restart there and not in `/tmp`.\n\
         </sandbox-restart-note>\n\n"
    )
}

/// Note prepended to a resumed turn when the workspace was rebuilt from a fresh
/// clone across the restart, so the agent knows its pre-restart scratch state
/// is gone rather than silently working against a reset tree.
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
/// Interrupted-run precedence gate for one Member: replays the interrupted run
/// only when no pending user message is waiting. Keys on `context`, never
/// `state.config`.
///
/// A probe error fails closed. Call once per Member per start:
/// [`open_for_work`] covers init, [`crate::task_context::register`] covers one
/// joining after Ready.
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
    } else {
        recover_interrupted_run(state, context, origin).await;
    }
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
            // Note text depends on `origin` (see `WorkspaceOrigin`).
            let prompt_text = match origin.restart_note(&context.checkout_dir) {
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

    #[test]
    fn persisted_origin_emits_no_restart_note() {
        assert_eq!(WorkspaceOrigin::Persisted.restart_note(WS), None);
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
        .restart_note(WS)
        .expect("snapshot restore must warn the agent");

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
        let note = WorkspaceOrigin::RestoredFromSnapshot { taken_at: None }
            .restart_note(WS)
            .expect("snapshot restore must warn the agent");

        assert!(note.contains("could not date that snapshot"), "{note}");
        assert!(note.contains("git status"), "{note}");
        assert!(!note.contains("intact and present"), "{note}");
    }

    /// The point of the tag: a flushed tree is reported as whole, so the agent
    /// resumes instead of spending a turn re-checking files nothing touched.
    #[test]
    fn a_final_flush_restore_reports_the_tree_whole_and_asks_for_no_audit() {
        let note = WorkspaceOrigin::RestoredFromFinalFlush
            .restart_note(WS)
            .expect("a restart still owes the agent a note");

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
            let note = origin.restart_note(WS).expect("a restore owes a note");
            assert!(note.contains(".env"), "{origin:?}: {note}");
        }
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
        assert!(wake_messages("task-a", BTreeMap::new(), &[]).is_empty());
    }

    #[test]
    fn a_wake_note_alone_produces_one_message() {
        let notes = BTreeMap::from([("task-a".to_string(), noted("the restart you asked for"))]);

        let messages = wake_messages("task-a", notes, &[]);
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

        let messages = wake_messages("task-a", notes, &lost);
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

        let messages = wake_messages("task-a", notes, &lost);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].0, "task-a");
        assert!(messages[0].1.contains("aaaaaaaa"));
        assert_eq!(messages[1].0, "task-b");
        assert!(messages[1].1.contains("task-b restarted"));
        assert!(!messages[1].1.contains("aaaaaaaa"));
    }
}
