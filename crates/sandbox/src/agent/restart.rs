use std::collections::BTreeMap;

use crate::agent::wake_note::{self, WakeNote};
use crate::init::InitPhase;
use crate::state::{SandboxState, TaskContext};
use tracing::{info, warn};

/// Cross the boundary from "restored but private" to "open for work", in the one
/// order that closes the startup precedence race:
///
/// 1. **Decide precedence while dispatch is still gated**, for every Member
///    registered so far. Session state was restored to disk in `run_core_init`
///    (before Ready). We probe for a pending Message and, only when none is
///    waiting, enqueue the interrupted-Run replay. External gRPC dispatch is
///    still a no-op here because init is not yet Ready.
/// 2. **Open the gate.** Flip `InitPhase::Ready` (health now returns 200).
/// 3. **Dispatch once, authoritatively.** Drain any pending Message — a no-op if
///    step 1 already enqueued a Recover, otherwise the Message that superseded it.
///
/// Doing step 1 before step 2 is what prevents a backend notification that races
/// init from fetching, submitting, and acknowledging the newer pending Message in
/// the window before the probe — which would leave the probe seeing no pending
/// row and enqueueing a now-stale Recover behind the Message.
pub(crate) async fn open_for_work(state: &SandboxState, workspace_origin: WorkspaceOrigin) {
    // Read and clear what a prior pod left behind before anything — a recovered
    // run, a new command — can overwrite it. The registry this pod started with
    // is empty, so these entries exist only on the workspace snapshot.
    let lost_commands =
        take_lost_commands(state, &state.config.task_id, &state.config.workspace_dir).await;
    let wake_notes = wake_note::take(&wake_note::default_path(&state.config.workspace_dir)).await;

    // Adopt whatever the restore or the clone produced as the upload baseline: a
    // workspace nobody has touched since is not worth shipping back.
    //
    // Consuming either file is the exception. The take deleted it here, but the
    // S3 archive it was restored from still carries it, so this tree is already
    // ahead of the object. Adopting it would let a turn that edits nothing skip
    // the upload, and the next replacement would restore the stale archive and
    // deliver the same wake again. Leaving the baseline unset makes the next
    // upload ship the removal.
    if state.config.workspace_sync_enabled && lost_commands.is_empty() && wake_notes.is_empty() {
        state.workspace_sync.adopt_current(&state.config).await;
    }

    {
        // Held across the recovery pass and the Ready flip, because
        // `task_context::register` takes the same lock and recovers a Member it
        // registers once Ready. A `JoinTask` racing this therefore either lands
        // before the snapshot below — and this pass recovers it — or blocks until
        // Ready and recovers itself. Drop the lock and a Member registering
        // between the snapshot and the flip is recovered by neither.
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

    // Awaited, not detached: dispatch only fetches, submits to the agent lifecycle task,
    // and returns (the turn itself runs in the coordinator) — it does not block on
    // the turn, so Phase 2 workspace setup is not delayed. Awaiting keeps this the
    // one authoritative startup dispatch and avoids an orphaned JoinHandle.
    crate::agent::dispatcher::dispatch_pending_messages(state.clone()).await;

    report_wake_notes(state, wake_notes, lost_commands).await;
}

/// Read and clear both copies of what a prior pod left unreported for one
/// Member, and merge them.
///
/// Both are cleared by the read, so the merge announces each command once and
/// no later restart announces it again.
///
/// Call this before the Member's registry can run a command. Each mirror write
/// is absolute, so the first `run` erases both copies this reads.
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

/// Union the two copies by command id.
///
/// The journal rides the workspace snapshot and the Sandbox row does not, so a
/// failed restore leaves only `stored` and a backend outage leaves only
/// `journal`. Either alone is a partial account of what the restart destroyed.
///
/// `journal` wins a collision: its write is under the registry's entries lock
/// and the row's is off it, so the journal cannot be the staler of the two.
fn merge_lost_commands(
    mut journal: Vec<crate::commands::JournaledCommand>,
    stored: Vec<crate::commands::JournaledCommand>,
) -> Vec<crate::commands::JournaledCommand> {
    let known: std::collections::HashSet<String> = journal.iter().map(|c| c.id.clone()).collect();
    journal.extend(stored.into_iter().filter(|c| !known.contains(&c.id)));
    journal
}

/// Wake each Member the replaced pod owes something to.
///
/// Two things go unsaid across a pod replacement. A `moat cmd` command's
/// process, its completion watch and its log live in memory and `/tmp`, so the
/// wake the agent was promised cannot fire. A caller that took the pod down
/// (`moat tasks restart --yes`) died with it, so it cannot report what it did.
/// Both ride the workspace snapshot instead, and this turns them into the wake
/// neither could deliver. The resumed session (`--continue` for Claude, restored
/// to disk in init) carries the context.
///
/// `dispatcher::queue_and_dispatch` owns the delivery, so the wake is a `user`
/// row the conversation shows and `find_interrupted_turn` can resume.
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

/// One message per Member owed a wake, each wrapped in one
/// `<sandbox-restart-note>`.
///
/// The lost-command text joins `own_task_id`'s note rather than travelling on
/// its own, so a self-restart that also lost commands opens one turn and not
/// two.
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

/// The wake a Member joining a replacement pod is owed for the commands that pod
/// destroyed, or `None` when it is owed none.
///
/// [`open_for_work`] covers the Member the pod booted with. A Member that joins
/// later has its own journal and its own `sandbox_unreported_commands` row, and
/// neither is read on any other path.
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

/// What a restart destroyed, or `None` when no command was owed a wake.
/// A command still `Running` at the snapshot was cut off mid-flight; a terminal
/// one had finished but the agent never read the outcome. Neither log survives,
/// so the text carries what the journal knew and tells the agent to re-run what
/// it still needs.
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
            format!("- `{}` ({}): {status}", c.id, c.label)
        })
        .collect();
    let noun = if lost.len() == 1 {
        "command was"
    } else {
        "commands were"
    };
    Some(format!(
        "The sandbox was restarted and {} background {noun} lost with it. Each was \
         started with `moat cmd`; its process and its log did not survive the \
         restart, so there is nothing left to read back:\n{}\n\n\
         Re-run any of these you still need, then continue the task.",
        lost.len(),
        lines.join("\n")
    ))
}

/// How the workspace came to exist on this start — determines what (if anything)
/// a resuming agent must be warned lost across the restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceOrigin {
    /// Repo persisted in place (EBS/PVC volume already held it) or this is a
    /// fresh task — nothing was lost, so no restart note is needed. The default,
    /// so [`SandboxState::workspace_origin`]'s constructors can write
    /// `Default::default()`.
    #[default]
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
/// Interrupted-run precedence gate for one Member: probe for pending user
/// messages first, and replay the interrupted run **only** when none are
/// waiting. A fresh user message supersedes the interrupted turn —
/// `dispatch_pending_messages` resumes the prior session with the new text as
/// the prompt instead. A probe error **fails closed** — treated as "pending
/// present", skipping the replay — so an unknown probe result can't resume the
/// stale Run only to have a newer Message dispatched concurrently once the
/// backend recovers.
///
/// Everything below keys on `context` and never on `state.config`, whose
/// `task_id` and `repo_dir` name the Member the pod booted with: in a shared
/// pod every other Member has its own interrupted Run and its own checkout.
///
/// Call this exactly once per Member per start — [`open_for_work`] covers the
/// Members registered at init, and [`crate::task_context::register`] covers one
/// that joins after Ready.
///
/// `pub(crate)` so the Sandbox-replacement lifecycle can be exercised in-crate
/// with an in-memory backend without driving the whole of `run_init`.
pub(crate) async fn recover_interrupted_run_unless_pending(
    state: &SandboxState,
    context: &TaskContext,
    origin: WorkspaceOrigin,
) {
    let task_id = &context.task_id;
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
        recover_interrupted_run(state, context, origin).await;
    }
}

/// Detect `context`'s interrupted agent run via the backend RPC and queue a
/// resume on its agent lifecycle task. Its one caller runs it once per Member
/// per start, **only when there are no new pending user messages** — a fresh
/// message supersedes the interrupted turn and `dispatch_pending_messages`
/// carries it forward instead.
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
            // Prepend a restart note so the resuming agent doesn't assume its
            // prior scratch state survived. Which note depends on how the
            // workspace was reconstituted (see `WorkspaceOrigin`).
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
