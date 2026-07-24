//! The harness-neutral coordinator.
//!
//! The coordinator drives a fixed [`Harness`] purely through its command/signal
//! seam: it submits input, cancels the active execution, and reacts to the
//! ordered [`Signal`] stream. It never sees a Claude or Codex handle and never
//! matches on the selected harness (ADR-0004, ADR-0005).
//!
//! What the coordinator owns (the invariants this module enforces):
//! - **Run identity** — a fresh Run id is minted on every
//!   [`Signal::ExecutionStarted`], for both input-opened and harness-internal
//!   executions, and stamped on every event and acknowledgement of that Run.
//! - **Acknowledgement** — a pending Message is acknowledged **only** after the
//!   harness reports [`Signal::InputAccepted`] for it (never on submit, never on
//!   `ExecutionStarted`), so an unaccepted input is never falsely confirmed.
//! - **Backend publication** — each [`Signal::Source`] is published as one
//!   per-block `agent.message` in the shape the backend assembler consumes
//!   (ADR-0002): a native block plus its `blockIndex`, grouped by `uuid`. The
//!   coordinator forwards blocks; it never assembles a Message.
//! - **Context-loss recovery** — a resume the harness reports as
//!   [`SessionImpact::Lost`] never silently drops the Message. The coordinator
//!   publishes the user-visible `session_context_lost` notice and re-drives one
//!   fresh recovery Turn (same logical Run) whose prompt tells the Agent to
//!   rebuild the prior conversation, keeping the pending ack alive. This is the
//!   seam owner of the "resume failure never silently creates a fresh Session"
//!   invariant — the harness refuses to start fresh; the coordinator decides to,
//!   explicitly and once.
//!
//! What this module deliberately does **not** yet own — those remain on the
//! legacy actor until it adopts this coordinator (the one migration-compat path,
//! removed in the final ticket): retry policy, credential recovery, wakeup and
//! background-watch scheduling, the circuit breaker, and `AgentState` status
//! transitions. Keeping this seam mapping small and independently tested is the
//! first, reversible step of that cutover.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use moatless_events::{AgentSubType, TaskTurnSubType};
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::agent::types::ResumeSession;
use crate::events::GrpcEventPublisher;

use super::handle::RunAck;
use super::harness::{
    Activity, ErrorClass, Harness, HarnessClosed, HarnessError, Input, InputId, Outcome, Rejection,
    Role, SessionImpact, Signal, SourceEvent,
};

/// How often a rate-limited liveness [`Signal::Activity`] is republished. The
/// reaper's idle floor only needs minute-level granularity against a TTL of tens
/// of minutes, and a chatty tool would otherwise flood the event stream.
const ACTIVITY_PUBLISH_INTERVAL: Duration = Duration::from_mins(1);

/// Drives one fixed [`Harness`], mapping its [`Signal`] stream onto Run
/// identity, acknowledgement, and backend publication.
pub struct Coordinator {
    harness: Harness,
    events: GrpcEventPublisher,
    task_id: String,
    /// The Run currently in flight. Minted on [`Signal::ExecutionStarted`] and
    /// stamped on every event and acknowledgement until the next one.
    run_id: String,
    /// The Session id reported by the most recent [`Signal::ExecutionStarted`].
    session_id: String,
    /// Acks awaiting their input's [`Signal::InputAccepted`], keyed by input id.
    pending_acks: HashMap<InputId, oneshot::Sender<RunAck>>,
    /// Submitted inputs awaiting acceptance or rejection, keyed by input id.
    /// Retained so a resume that the harness reports as [`SessionImpact::Lost`]
    /// can be re-driven as a fresh context-recovery Turn instead of being dropped.
    pending_inputs: HashMap<InputId, Input>,
    /// Next `blockIndex` per native Message id, reset at each Run start. Lets the
    /// backend assemble per-block source events in native order (ADR-0002).
    block_indices: HashMap<String, usize>,
    /// When the last rate-limited liveness event was published.
    last_activity_publish: Option<Instant>,
    /// Reuse the current `run_id` on the next [`Signal::ExecutionStarted`] instead
    /// of minting a fresh one. Set when a context-loss recovery mints the Run for
    /// its notice, so the recovered Turn continues the same logical Run.
    hold_run_id: bool,
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
            pending_inputs: HashMap::new(),
            block_indices: HashMap::new(),
            last_activity_publish: None,
            hold_run_id: false,
        }
    }

    /// Submit input and register its acknowledgement. The ack resolves only once
    /// the harness reports the input accepted (see [`Signal::InputAccepted`]).
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

    /// Consume the harness signal stream until it stops and drains.
    pub async fn run(&mut self) {
        while let Some(signal) = self.harness.next_signal().await {
            self.handle_signal(signal).await;
        }
    }

    /// Drive signals until one execution reaches its terminal signal
    /// (`ExecutionFinished` / `InputRejected`), then stop. A persistent harness
    /// parks awaiting the next input rather than closing, so the always-on
    /// [`Self::run`] loop would never return within a single-turn test; this
    /// bounds the loop deterministically instead.
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

    /// Map one signal onto Run/ack/publication effects.
    async fn handle_signal(&mut self, signal: Signal) {
        match signal {
            Signal::ExecutionStarted { session, .. } => {
                // A new execution opens a new Run — for input-opened and
                // harness-internal executions alike. The acknowledgement (if any)
                // waits for the following `InputAccepted`. A context-loss recovery
                // already minted the Run for its notice, so the recovered Turn
                // continues that same logical Run rather than opening a second.
                self.session_id = session.0;
                if !self.hold_run_id {
                    self.run_id = new_run_id();
                }
                self.hold_run_id = false;
                self.block_indices.clear();
                self.publish_turn_start().await;
                self.run_events()
                    .publish_agent_event(
                        &self.task_id,
                        AgentSubType::SessionInit,
                        json!({ "session_id": self.session_id }),
                    )
                    .await;
            }
            Signal::InputAccepted { input } => {
                // Accepted: it will never need context-loss recovery, so release
                // the retained copy. Acknowledge only now, against the current Run,
                // so the batch joins this Run's turn regardless of ack/event
                // delivery order.
                self.pending_inputs.remove(&input);
                if let Some(ack) = self.pending_acks.remove(&input) {
                    let run_ack = RunAck {
                        session_id: self.session_id.clone(),
                        run_id: self.run_id.clone(),
                    };
                    // A closed receiver means the dispatcher gave up waiting for
                    // this batch (e.g. it timed out and will redispatch). The
                    // acknowledgement-after-acceptance invariant still held; log
                    // the drop with the input/Run identity so it stays diagnosable.
                    if ack.send(run_ack).is_err() {
                        tracing::warn!(
                            input = %input.0,
                            run_id = %self.run_id,
                            "acceptance ack dropped: receiver closed before delivery"
                        );
                    }
                }
            }
            Signal::Source(event) => {
                let block_index = self.next_block_index(&event.message_id.0);
                self.run_events()
                    .publish_agent_event(
                        &self.task_id,
                        AgentSubType::Message,
                        message_payload(&event, block_index),
                    )
                    .await;
            }
            Signal::Activity(activity) => self.publish_activity(&activity).await,
            Signal::ExecutionFinished { outcome } => self.publish_outcome(outcome).await,
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

    async fn publish_outcome(&self, outcome: Outcome) {
        match outcome {
            Outcome::Completed => {
                self.run_events()
                    .publish_agent_event(&self.task_id, AgentSubType::Completed, json!({}))
                    .await;
            }
            // A cancelled execution ends the turn without its own terminal event,
            // matching the legacy cancel path; the caller-side cancel flow closes
            // the turn.
            Outcome::Cancelled => {}
            Outcome::Failed(error) => self.publish_error(&error).await,
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
        resume: None,
        start_fresh: true,
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
fn context_recovery_prompt(task_id: &str, prompt: &str) -> String {
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

/// Wrap one native block into the per-block `agent.message` payload the backend
/// assembler consumes (ADR-0002): the single-block `content` array plus the
/// `blockIndex` and `uuid` it merges on, and the `session_id` /
/// `parent_tool_use_id` it groups by.
fn message_payload(event: &SourceEvent, block_index: usize) -> Value {
    let mut message = json!({
        "role": role_str(event.role),
        "id": event.message_id.0,
        "content": [event.block.clone()],
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
        ErrorClass::UnavailableSession => "unavailable_session",
        ErrorClass::InvalidConfiguration => "invalid_configuration",
        ErrorClass::InvalidInput => "invalid_input",
        ErrorClass::NativeProtocol => "native_protocol",
        ErrorClass::ProcessExit => "process_exit",
        ErrorClass::UnresponsiveProcess => "unresponsive_process",
        ErrorClass::Internal => "internal",
    }
}

#[cfg(test)]
mod tests;
