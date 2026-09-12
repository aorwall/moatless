//! Maps one [`Harness`]'s signal stream onto Run events and acknowledgements.
//!
//! It drives a fixed [`Harness`] through its command/signal seam alone: it
//! never sees a Claude or Codex handle and never matches on the selected
//! harness.
//!
//! Invariants it enforces:
//! - A fresh Run id is minted on every [`Signal::ExecutionStarted`] and stamped
//!   on every event and acknowledgement of that Run.
//! - A Message opening an execution is acknowledged only after
//!   [`Signal::InputAccepted`]; one injected into a running execution only after
//!   a later non-terminal signal proves the execution continued past the
//!   injection. On a terminal signal the ack is dropped, so the backend
//!   redispatches.
//! - Each [`Signal::Source`] is published as one per-block `agent.message`
//!   grouped by `uuid`. The coordinator forwards blocks; it never assembles a
//!   Message.
//! - A resume reported as [`SessionImpact::Lost`] publishes the
//!   `session_context_lost` notice and re-drives one recovery Turn under the
//!   same Run, keeping the pending ack alive. The harness refuses to start
//!   fresh; the coordinator decides to, once.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moatless_types::agent::AgentSubType;
use moatless_types::task::TaskTurnSubType;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::agent::agent_state::AgentState;
use crate::agent::handle::RunAck;
use crate::agent::harness::content::CanonicalContentBlock;
use crate::agent::harness::{
    Activity, Cause, ContextWindow, ErrorClass, Harness, HarnessClosed, HarnessError, Input,
    InputId, Outcome, Rejection, Role, SessionImpact, Signal, SourceEvent,
};
use crate::agent::prompt::PromptToolName;
use crate::agent::types::ResumeSession;
use crate::agent::wakeup_job::WakeupJob;
use crate::events::GrpcEventPublisher;

/// How often a rate-limited liveness [`Signal::Activity`] is republished. The
/// reaper's idle floor only needs minute-level granularity against a TTL of tens
/// of minutes, and a chatty tool would otherwise flood the event stream.
const ACTIVITY_PUBLISH_INTERVAL: Duration = Duration::from_mins(1);

/// Drives one fixed [`Harness`], mapping its [`Signal`] stream onto Run
/// identity, acknowledgement, and backend publication.
pub struct Coordinator {
    harness: Harness,
    pub(super) events: GrpcEventPublisher,
    task_id: String,
    /// The Run currently in flight. Minted on [`Signal::ExecutionStarted`] and
    /// stamped on every event and acknowledgement until the next one.
    run_id: String,
    /// The Session id reported by the most recent [`Signal::ExecutionStarted`].
    session_id: String,
    /// Acks awaiting their input's [`Signal::InputAccepted`], keyed by input id.
    pending_acks: HashMap<InputId, oneshot::Sender<RunAck>>,
    /// The input that opened the current execution. Its ack can resolve as soon
    /// as it is accepted; accepted inputs not matching this joined an active
    /// execution and wait for later non-terminal output before acking.
    opening_input: Option<InputId>,
    /// Accepted inputs injected into the active execution. They stay unacked
    /// until the harness emits later non-terminal output (`Source` / `Activity`);
    /// if the next signal is terminal, the senders are dropped so the dispatcher
    /// leaves the backend batch unacknowledged for redispatch.
    accepted_injected_acks: Vec<(InputId, oneshot::Sender<RunAck>)>,
    /// Submitted inputs awaiting acceptance or rejection, keyed by input id.
    /// Retained so a resume that the harness reports as [`SessionImpact::Lost`]
    /// can be re-driven as a fresh context-recovery Turn instead of being dropped.
    pending_inputs: HashMap<InputId, Input>,
    /// Next `blockIndex` per native Message id, reset at each Run start. Lets the
    /// backend assemble per-block source events in native order (server-owned assembly).
    block_indices: HashMap<String, usize>,
    /// When the last rate-limited liveness event was published.
    last_activity_publish: Option<Instant>,
    /// Reuse the current `run_id` on the next [`Signal::ExecutionStarted`] instead
    /// of minting a fresh one. Set when a context-loss recovery mints the Run for
    /// its notice, so the recovered Turn continues the same logical Run.
    hold_run_id: bool,
    /// One-shot: suppress the terminal `error` event for the next `Failed`
    /// outcome. The service sets this when it has decided to retry a transient
    /// failure, so the user sees `retrying` (published by the service) instead of
    /// a terminal error.
    suppress_failure_event: bool,
    /// The state machine this coordinator drives.
    ///
    /// Held here so a signal reaches the state machine and the event stream as
    /// one act; a turn open in one and not the other is what the backend then
    /// has to reconcile. It also records an interactive call as published,
    /// whichever route published it, so one call writes one row.
    ///
    /// `None` in tests that drive a coordinator with no agent behind it.
    agent_state: Option<Arc<AgentState>>,
    /// Root of the run's cancellation tree, from which each turn's token is
    /// minted. Bound by [`super::service::CoordinatorService::new`], which owns the root.
    /// `None` for the same coordinators `agent_state` is `None` for.
    run_cancel: Option<CancellationToken>,
    /// Whether a turn is open, as the same act that opened or closed it left
    /// it. The service reads it rather than tracking its own copy.
    turn_active: bool,
}

impl Coordinator {
    /// Build a coordinator over a constructed [`Harness`] and a run-agnostic
    /// publisher.
    pub fn new(harness: Harness, events: GrpcEventPublisher, task_id: impl Into<String>) -> Self {
        Self {
            harness,
            events,
            task_id: task_id.into(),
            run_id: String::new(),
            session_id: String::new(),
            pending_acks: HashMap::new(),
            opening_input: None,
            accepted_injected_acks: Vec::new(),
            pending_inputs: HashMap::new(),
            block_indices: HashMap::new(),
            last_activity_publish: None,
            hold_run_id: false,
            suppress_failure_event: false,
            agent_state: None,
            run_cancel: None,
            turn_active: false,
        }
    }

    /// Share the state machine with a coordinator that has no service behind
    /// it — the permission-dedup tests. A service binds it itself, along with
    /// the cancellation root a turn transition also needs; see [`Self::bind`].
    #[must_use]
    #[cfg(test)]
    pub fn driving(mut self, agent_state: Arc<AgentState>) -> Self {
        self.agent_state = Some(agent_state);
        self
    }

    /// Bind the state machine and the run's cancellation root.
    ///
    /// Called by [`super::service::CoordinatorService::new`] with the very `AgentState` the
    /// service holds, so a coordinator and its service can never end up driving
    /// two different state machines — which is the disagreement this pairing
    /// exists to prevent. Binding the cancellation root here too means the
    /// turn's token is minted where the turn is opened.
    pub(crate) fn bind(&mut self, agent_state: Arc<AgentState>, run_cancel: CancellationToken) {
        self.agent_state = Some(agent_state);
        self.run_cancel = Some(run_cancel);
    }

    /// Whether a turn is currently open.
    pub(crate) fn turn_active(&self) -> bool {
        self.turn_active
    }

    /// Submit input and register its acknowledgement. Opening-turn inputs ack
    /// on [`Signal::InputAccepted`]; inputs injected into an active turn ack
    /// only after later non-terminal output from that turn.
    pub async fn submit(
        &mut self,
        input: Input,
        ack: oneshot::Sender<RunAck>,
    ) -> Result<(), HarnessClosed> {
        self.pending_acks.insert(input.id.clone(), ack);
        self.pending_inputs.insert(input.id.clone(), input.clone());
        self.harness.submit(input).await
    }

    /// Cancel the active execution. Idempotent (see [`Harness::cancel`]).
    pub fn cancel(&self) {
        self.harness.cancel();
    }

    /// Drive signals until one execution reaches its terminal signal
    /// (`ExecutionFinished` / `InputRejected`), then stop. A persistent harness
    /// parks awaiting the next input rather than closing, so draining the stream
    /// to its end would never return within a single-turn test; this bounds the
    /// loop deterministically instead.
    #[cfg(test)]
    pub(crate) async fn pump_one_execution(&mut self) {
        while let Some(signal) = self.harness.next_signal().await {
            let terminal = matches!(
                signal,
                Signal::ExecutionFinished { .. } | Signal::InputRejected { .. }
            );
            self.handle_signal(signal).await;
            if terminal {
                return;
            }
        }
    }

    /// Receive the next normalized signal, or `None` once the harness has stopped.
    pub(crate) async fn next_signal(&mut self) -> Option<Signal> {
        self.harness.next_signal().await
    }

    /// The Run id currently in flight (minted on the most recent
    /// [`Signal::ExecutionStarted`]).
    pub(crate) fn current_run_id(&self) -> &str {
        &self.run_id
    }

    /// Whether a context-loss recovery is in flight — the coordinator re-drove a
    /// fresh Turn after a lost resume and is holding the Run for it. The service
    /// reads this to avoid recording a terminal error for a rejection the
    /// coordinator already recovered (which would otherwise pollute the
    /// consecutive-error count the dispatcher and circuit breaker read).
    pub(crate) fn is_recovering(&self) -> bool {
        self.hold_run_id
    }

    /// Keep the current Run id on the next `ExecutionStarted` instead of minting a
    /// fresh one — used when re-driving the same Turn (a transient-error retry),
    /// so the retried attempt continues the same Run the backend maps to the Turn.
    pub(crate) fn reuse_run_on_next_start(&mut self) {
        self.hold_run_id = true;
    }

    /// Resume an interrupted turn under its original Run id. The next
    /// `ExecutionStarted` adopts `run_id` rather than minting a fresh one, so the
    /// recovery run's events (and its terminal close) map back to the interrupted
    /// turn instead of forking a new one that the reaper would later orphan.
    pub(crate) fn resume_run(&mut self, run_id: String) {
        self.run_id = run_id;
        self.hold_run_id = true;
    }

    /// Suppress the terminal `error` event for the next `Failed` outcome. The
    /// service calls this when it has classified the failure as a retryable
    /// transient error with retry budget remaining, so the coordinator stays
    /// silent and the service publishes `retrying` instead.
    pub(crate) fn suppress_next_failure_event(&mut self) {
        self.suppress_failure_event = true;
    }

    /// Map one signal onto Run/ack/state/publication effects.
    ///
    /// Returns the cancellation token of a turn this signal opened, for the run
    /// loop to adopt as its cancellation arm. `None` for every other signal, and
    /// for a coordinator with no state machine behind it.
    pub(crate) async fn handle_signal(&mut self, signal: Signal) -> Option<CancellationToken> {
        let mut opened = None;
        match signal {
            Signal::ExecutionStarted {
                session,
                agent_turn_id,
                cause,
            } => {
                self.drop_accepted_injected_acks(
                    "new execution started before non-terminal output",
                );
                // A new execution opens a new Run — for input-opened and
                // harness-internal executions alike. The acknowledgement (if any)
                // waits for the following `InputAccepted`. A context-loss recovery
                // already minted the Run for its notice, so the recovered Turn
                // continues that same logical Run rather than opening a second.
                self.session_id = session.0;
                if !self.hold_run_id {
                    self.run_id = new_run_id();
                }
                self.opening_input = match cause {
                    Cause::Input(input) => Some(input),
                    Cause::Internal => None,
                };
                self.hold_run_id = false;
                self.block_indices.clear();
                // The state machine enters the turn *before* the events say it
                // has: a reader that catches the gap sees a not-yet-started
                // turn, never a started turn whose agent reports idle.
                opened = self.open_turn().await;
                self.publish_turn_start().await;
                self.run_events()
                    .publish_agent_event(
                        &self.task_id,
                        AgentSubType::SessionInit,
                        json!({
                            "session_id": self.session_id,
                            // Codex only; the backend records it on the turn so a
                            // fork can cut the thread here. Omitted when absent.
                            "agent_turn_id": agent_turn_id,
                        }),
                    )
                    .await;
            }
            Signal::InputAccepted { input } => {
                // Accepted: it will never need context-loss recovery, so release
                // the retained copy. An input that opened this execution acks at
                // acceptance. An input injected into an already-running execution
                // waits until the agent emits later non-terminal output; if the
                // execution is already at its terminal result, that ack is dropped
                // so the backend can redispatch the still-unhandled batch.
                self.pending_inputs.remove(&input);
                if let Some(ack) = self.pending_acks.remove(&input) {
                    if self.opening_input.as_ref() == Some(&input) {
                        self.opening_input = None;
                        self.send_run_ack(input, ack, "acceptance");
                    } else {
                        self.accepted_injected_acks.push((input, ack));
                    }
                } else if self.opening_input.as_ref() == Some(&input) {
                    self.opening_input = None;
                }
            }
            Signal::Source(event) => {
                // The index is spent either way, so the blocks that follow keep
                // the positions the model gave them.
                let block_index = self.next_block_index(&event.message_id.0);
                if !self.published_as_a_prompt(&event.block).await {
                    self.run_events()
                        .publish_agent_event(
                            &self.task_id,
                            AgentSubType::Message,
                            message_payload(&event, block_index),
                        )
                        .await;
                }
                self.ack_accepted_injected_inputs("source");
            }
            Signal::Activity(activity) => {
                self.publish_activity(&activity).await;
                self.ack_accepted_injected_inputs("activity");
            }
            Signal::ContextWindow(window) => self.publish_context_window(window).await,
            Signal::ExecutionFinished { outcome } => {
                self.drop_accepted_injected_acks("execution finished before non-terminal output");
                // Leave the turn *before* the events say it ended, for the same
                // reason opening enters it first: the observable gap must be
                // the harmless direction. This is the pairing whose absence put
                // task `0d48056e` in the state the reaper had no arm for — the
                // agent reporting `running` under a turn the events had closed.
                self.close_turn(&outcome).await;
                self.publish_outcome(outcome).await;
            }
            Signal::InputRejected { input, reason } => {
                let original = self.pending_inputs.remove(&input);
                match reason {
                    // A resume the harness could not load (`SessionImpact::Lost`)
                    // must never silently drop the Message. Re-drive it once as a
                    // fresh context-recovery Turn, keeping the pending ack alive
                    // for the recovered Turn's acceptance.
                    Rejection::Failed(err)
                        if original
                            .as_ref()
                            .is_some_and(|input| is_lost_resume(input, &err)) =>
                    {
                        // `is_some_and` above proved `original` is present.
                        if let Some(original) = original {
                            self.recover_lost_session(input, &original, &err).await;
                        }
                    }
                    // No `ExecutionStarted` preceded a non-recoverable rejection,
                    // so no Run was minted and there is nothing to close. Drop the
                    // ack (the dispatcher observes a closed channel and leaves the
                    // batch unacknowledged) and surface a classified failure. This
                    // also ends a recovery re-drive that itself failed — recovery
                    // is bounded to a single attempt.
                    Rejection::Failed(err) => {
                        self.hold_run_id = false;
                        drop(self.pending_acks.remove(&input));
                        self.publish_error(&err).await;
                    }
                    Rejection::Cancelled => {
                        self.hold_run_id = false;
                        drop(self.pending_acks.remove(&input));
                    }
                }
            }
        }
        opened
    }

    /// Move the state machine into the turn this signal opened, and mint that
    /// turn's cancellation token — so `agent_state.stop()` cancels the token,
    /// which the run loop's cancellation arm turns into a harness cancel.
    ///
    /// `None` when there is no state machine behind this coordinator, which is
    /// also when there is no turn to cancel.
    async fn open_turn(&mut self) -> Option<CancellationToken> {
        let (Some(state), Some(run_cancel)) = (&self.agent_state, &self.run_cancel) else {
            return None;
        };
        let turn_cancel = run_cancel.child_token();
        state.set_session_id(self.session_id.clone()).await;
        state.set_run_id(self.run_id.clone()).await;
        state.set_running(turn_cancel.clone()).await;
        self.turn_active = true;
        Some(turn_cancel)
    }

    /// Move the state machine out of the turn this outcome ended.
    ///
    /// Only the status is decided here; retry budget, the breaker, scheduled
    /// jobs and workspace sync stay the service's. A failure is the exception —
    /// the service settles the status before handing the signal over, so this
    /// leaves it alone. Either way the turn is left before any event says it
    /// ended.
    async fn close_turn(&mut self, outcome: &Outcome) {
        self.turn_active = false;
        let Some(state) = &self.agent_state else {
            return;
        };
        match outcome {
            Outcome::Completed { .. } => state.set_idle().await,
            Outcome::Cancelled => state.set_stopped().await,
            Outcome::Failed(_) => {}
        }
    }

    /// Publisher scoped to the Run currently in flight.
    fn run_events(&self) -> GrpcEventPublisher {
        self.events.for_run(&self.run_id)
    }

    /// Position of the next block in its native Message, advanced per id.
    fn next_block_index(&mut self, message_id: &str) -> usize {
        let slot = self
            .block_indices
            .entry(message_id.to_string())
            .or_insert(0);
        let index = *slot;
        *slot += 1;
        index
    }

    /// Whether the permission handler already published this block's call.
    ///
    /// Only the calls it parks on are asked about: those are announced from the
    /// handler as it waits, because a person cannot answer a question they were
    /// never shown. The block itself still arrives here, carrying the same
    /// tool-use id, and the conversation keeps one row per id — so exactly one
    /// of the two publishes it, and it is whichever got here first.
    async fn published_as_a_prompt(&self, block: &CanonicalContentBlock) -> bool {
        let (Some(state), CanonicalContentBlock::ToolUse { id, name, .. }) =
            (&self.agent_state, block)
        else {
            return false;
        };
        if PromptToolName::from_tool_name(name).is_none() {
            return false;
        }
        !state.claim_prompt_publication(id).await
    }

    fn send_run_ack(&self, input: InputId, ack: oneshot::Sender<RunAck>, reason: &'static str) {
        let run_ack = RunAck {
            session_id: self.session_id.clone(),
            run_id: self.run_id.clone(),
        };
        // A closed receiver means the dispatcher gave up waiting for this batch
        // (e.g. it timed out and will redispatch). The acknowledgement boundary
        // still held; log the drop with the input/Run identity so it stays
        // diagnosable.
        if ack.send(run_ack).is_err() {
            tracing::warn!(
                input = %input.0,
                run_id = %self.run_id,
                reason,
                "run ack dropped: receiver closed before delivery"
            );
        }
    }

    fn ack_accepted_injected_inputs(&mut self, reason: &'static str) {
        let acks = std::mem::take(&mut self.accepted_injected_acks);
        for (input, ack) in acks {
            self.send_run_ack(input, ack, reason);
        }
    }

    fn drop_accepted_injected_acks(&mut self, reason: &'static str) {
        if self.accepted_injected_acks.is_empty() {
            return;
        }
        let count = self.accepted_injected_acks.len();
        self.accepted_injected_acks.clear();
        tracing::warn!(
            count,
            run_id = %self.run_id,
            reason,
            "dropping accepted injected input acks"
        );
    }

    async fn publish_turn_start(&self) {
        let events = self.run_events();
        events
            .publish_agent_event(&self.task_id, AgentSubType::Started, json!({}))
            .await;
        events
            .publish_task_turn_event(&self.task_id, TaskTurnSubType::Active, json!({}))
            .await;
    }

    async fn publish_activity(&mut self, activity: &Activity) {
        let due = self
            .last_activity_publish
            .is_none_or(|at| at.elapsed() >= ACTIVITY_PUBLISH_INTERVAL);
        if !due {
            return;
        }
        self.last_activity_publish = Some(Instant::now());
        self.run_events()
            .publish_agent_event(
                &self.task_id,
                AgentSubType::Activity,
                json!({ "detail": activity.detail }),
            )
            .await;
    }

    /// One context-window reading, as its own event.
    ///
    /// Not rate-limited the way liveness is: a reading is what the meter shows,
    /// and dropping one leaves the meter stale. Codex reports per model request,
    /// so several can arrive in a turn — the projection keeps the newest per turn
    /// on the way out rather than the writer keeping fewer.
    async fn publish_context_window(&mut self, window: ContextWindow) {
        self.run_events()
            .publish_agent_event(
                &self.task_id,
                AgentSubType::ContextWindow,
                json!({
                    "usedTokens": window.used_tokens,
                    "maxTokens": window.max_tokens,
                }),
            )
            .await;
    }

    async fn publish_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Completed { totals } => {
                // The terminal carries the run's authoritative token/cost totals
                // when the harness reported them; an older runtime that reported
                // none still publishes so the turn closes, just with an empty body.
                let payload = totals.map_or_else(|| json!({}), |t| t.to_payload());
                self.run_events()
                    .publish_agent_event(&self.task_id, AgentSubType::Completed, payload)
                    .await;
            }
            // A cancelled execution ends the turn without its own terminal event;
            // the caller-side cancel flow closes the turn.
            Outcome::Cancelled => {}
            Outcome::Failed(error) => {
                // A retry the service already scheduled suppresses the terminal
                // error event (one-shot); the service publishes `retrying`.
                if self.suppress_failure_event {
                    self.suppress_failure_event = false;
                } else {
                    self.publish_error(&error).await;
                }
            }
        }
    }

    async fn publish_error(&self, error: &HarnessError) {
        self.run_events()
            .publish_agent_event(
                &self.task_id,
                AgentSubType::Error,
                json!({
                    "error": error.diagnostic,
                    "reason": error_reason(error.class),
                }),
            )
            .await;
    }

    /// Drive a single context-loss recovery for a resume the harness reported as
    /// [`SessionImpact::Lost`]. Mints the Run for the recovery Turn (held so its
    /// `ExecutionStarted` continues it), publishes the user-visible
    /// `session_context_lost` notice, and re-submits a fresh Turn whose prompt
    /// instructs the Agent to recover the prior conversation. The pending ack is
    /// left registered, so the recovered Turn's acceptance resolves the original
    /// Message. The recovery input carries no resume, so a further failure can
    /// never re-enter recovery — the attempt is bounded to one.
    async fn recover_lost_session(&mut self, input: InputId, original: &Input, err: &HarnessError) {
        self.run_id = new_run_id();
        self.hold_run_id = true;
        self.run_events()
            .publish_agent_event(
                &self.task_id,
                AgentSubType::SessionContextLost,
                json!({
                    "stale_session_id": stale_session_id(&original.resume),
                    "error": err.diagnostic,
                    "reason": "transcript_unavailable",
                }),
            )
            .await;

        let recovery = recovery_input(original, &self.task_id);
        self.pending_inputs.insert(input.clone(), recovery.clone());
        if self.harness.submit(recovery).await.is_err() {
            // The harness closed mid-recovery: nothing left to drive, so release
            // the retained input and let the ack channel close.
            self.hold_run_id = false;
            self.pending_inputs.remove(&input);
            drop(self.pending_acks.remove(&input));
        }
    }
}

/// Whether a rejected input was a resume attempt whose Session the harness could
/// not load — the one condition that turns a rejection into a context-loss
/// recovery rather than a plain failure.
fn is_lost_resume(input: &Input, err: &HarnessError) -> bool {
    err.session_impact == SessionImpact::Lost && input.resume.is_some() && !input.start_fresh
}

/// Build the fresh recovery Turn for a lost resume: same Turn configuration, but
/// starting a new Session (`start_fresh`, no resume) with a prompt that tells the
/// Agent to recover the prior conversation before acting.
fn recovery_input(original: &Input, task_id: &str) -> Input {
    Input {
        id: original.id.clone(),
        text: context_recovery_prompt(task_id, &original.text),
        model: original.model.clone(),
        mode: original.mode.clone(),
        effort: original.effort.clone(),
        resume: None,
        start_fresh: true,
        // A recovery prompt opens a new Session outright; there is no cut to
        // branch, and the Task's own session already exists.
        fork: None,
        env: original.env.clone(),
        system_prompt: original.system_prompt.clone(),
    }
}

/// The stale Session id a lost resume was targeting, for the `session_context_lost`
/// notice. A `--continue` (`Last`) resume names no id.
fn stale_session_id(resume: &Option<ResumeSession>) -> String {
    match resume {
        Some(ResumeSession::WithId(id)) => id.clone(),
        _ => String::new(),
    }
}

/// Wrap a prompt so the fresh recovery Session rebuilds the lost conversation
/// context via the `moat` CLI before acting. The prior transcript is still
/// readable through `moat messages list` even though it is no longer in the
/// resumed CLI Session.
pub(super) fn context_recovery_prompt(task_id: &str, prompt: &str) -> String {
    format!(
        "<context-recovery>\n\
         Your previous session for this task could not be resumed, so the prior \
         conversation is not in your context. Before acting, recover it by running:\n\
         \x20 moat messages list {task_id} --full --limit 100\n\
         (if it reports older messages, rerun with the suggested --before-id to page back)\n\
         Treat that transcript as the conversation so far and continue from it.\n\
         </context-recovery>\n\n\
         {prompt}"
    )
}

/// Mint an identifier for a new agent Run. A UUID (not a counter) so it stays
/// unique across pod restarts without a per-instance qualifier.
fn new_run_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Payload for the `taskTurn.turn_interrupted` published when the pod's own
/// shutdown cancels a Turn nobody asked to end (SIGTERM from a node drain, spot
/// reclaim, eviction, or a lifecycle restart).
///
/// It deliberately does **not** publish `agent.stopped`: a terminal
/// `agent`/`taskTurn` `completed`/`error`/`stopped` row is exactly what
/// `find_interrupted_turn` treats as "this Turn ended", so publishing one here
/// would tell the replacement pod there is nothing to resume — the Turn would
/// sit half-finished until a human noticed and said "carry on". Mirrors the
/// message the pod watcher emits for the ungraceful (SIGKILL) case, so both
/// interruption paths read the same in the UI.
pub(super) fn interrupted_turn_payload() -> Value {
    json!({
        "message": "Turn interrupted — the sandbox pod was shut down mid-turn. \
                    Resuming when it reschedules.",
        "reason": "sandbox_shutdown",
    })
}

/// Extract a [`WakeupJob`] from a canonical Source block when it is a
/// `ScheduleWakeup` `tool_use`.
pub(super) fn extract_wakeup_from_block(block: &CanonicalContentBlock) -> Option<WakeupJob> {
    let CanonicalContentBlock::ToolUse { id, name, input } = block else {
        return None;
    };
    if name != "ScheduleWakeup" {
        return None;
    }
    WakeupJob::from_tool_use(id, input)
}

/// Wrap one native block into the per-block `agent.message` payload the backend
/// assembler consumes (server-owned assembly): the single-block `content` array plus the
/// `blockIndex` and `uuid` it merges on, and the `session_id` /
/// `parent_tool_use_id` it groups by.
fn message_payload(event: &SourceEvent, block_index: usize) -> Value {
    let mut message = json!({
        "role": role_str(event.role),
        "id": event.message_id.0,
        "content": [event.block_json()],
    });
    if let Some(model) = &event.model {
        message["model"] = json!(model);
    }
    if let Some(usage) = &event.usage {
        message["usage"] = usage.clone();
    }

    let mut payload = json!({
        "uuid": event.message_id.0,
        "blockIndex": block_index,
        "session_id": event.session.0,
        "message": message,
    });
    if let Some(parent) = &event.parent_tool_use_id {
        payload["parent_tool_use_id"] = json!(parent);
    }
    // A tool_result block carries its typed result at the payload top level, where
    // the backend reads it (`toolUseResult`) to enrich the UI's rendered output.
    if let Some(tool_use_result) = &event.tool_use_result {
        payload["toolUseResult"] = tool_use_result.clone();
    }
    payload
}

fn role_str(role: Role) -> &'static str {
    match role {
        Role::Assistant => "assistant",
        Role::User => "user",
        Role::System => "system",
    }
}

/// A stable snake-case tag for the error class, carried on `agent.error` for
/// observability alongside the diagnostic.
fn error_reason(class: ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication => "authentication",
        ErrorClass::RateLimit => "rate_limit",
        ErrorClass::Connectivity => "connectivity",
        ErrorClass::UnavailableSession => "unavailable_session",
        ErrorClass::InvalidConfiguration => "invalid_configuration",
        ErrorClass::InvalidInput => "invalid_input",
        ErrorClass::NativeProtocol => "native_protocol",
        ErrorClass::ProcessExit => "process_exit",
        ErrorClass::UnresponsiveProcess => "unresponsive_process",
        ErrorClass::Internal => "internal",
    }
}
