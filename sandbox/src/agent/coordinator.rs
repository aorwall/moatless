//! The harness-neutral coordinator.
//!
//! The coordinator drives a fixed [`Harness`] purely through its command/signal
//! seam: it submits input, cancels the active execution, and reacts to the
//! ordered [`Signal`] stream. It never sees a Claude or Codex handle and never
//! matches on the selected harness (the fixed-harness decision, the harness-outcome boundary).
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
//!   (server-owned assembly): a native block plus its `blockIndex`, grouped by `uuid`. The
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
//! The harness-neutral [`Coordinator`] above owns only the seam mapping. The
//! production lifecycle policy lives in
//! [`CoordinatorService`] (also in this module): transient-error **retry** with
//! backoff (same Run id), the consecutive-error **circuit breaker**, **wakeup
//! and background-watch scheduling** (armed from Source blocks at turn end),
//! **credential-recovery retry** (pull fresh credentials and re-drive the same
//! prompt/Run after a stale-credential failure, or surface one reconnect error
//! when nothing fresher is available), and **`AgentState` status transitions**.
//! `handle::spawn_agent` now routes every valid AGENT_TYPE pod through
//! [`CoordinatorService`]; missing or unsupported selections are classified as
//! invalid configuration at startup.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moatless_api_schemas::content::CanonicalContentBlock;
use moatless_events::{AgentSubType, SandboxStatus, TaskTurnSubType};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::agent::heartbeat::{Heartbeat, HEARTBEAT_TICK_INTERVAL};
use crate::agent::types::{ResumeSession, StopSource};
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::credentials::Dispatcher;
use crate::events::GrpcEventPublisher;
use crate::init::credentials::filter_agent_credentials;
use crate::init::workspace_sync;

use super::agent_state::AgentState;
use super::background_watch_job::BackgroundWatchJob;
use super::handle::{
    credential_recovery::{try_recover_credentials, MAX_CREDENTIAL_RECOVERY_RETRIES},
    resolve_agent_harness,
    retry::{compute_backoff, is_retryable_harness_error, MAX_RETRIES},
    AgentCommand, RunAck, AGENT_TYPE_CLAUDE_CODE,
};
use super::harness::{
    Activity, ContextWindow, ErrorClass, Harness, HarnessClosed, HarnessError, Input, InputId,
    Outcome, Rejection, Role, SessionImpact, Signal, SourceEvent,
};
use super::scheduled_job::{JobKind, ScheduledJobRegistry};
use super::wakeup_job::WakeupJob;

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
            suppress_failure_event: false,
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

    /// Suppress the terminal `error` event for the next `Failed` outcome. The
    /// service calls this when it has classified the failure as a retryable
    /// transient error with retry budget remaining, so the coordinator stays
    /// silent and the service publishes `retrying` instead.
    pub(crate) fn suppress_next_failure_event(&mut self) {
        self.suppress_failure_event = true;
    }

    /// Map one signal onto Run/ack/publication effects.
    pub(crate) async fn handle_signal(&mut self, signal: Signal) {
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
            Signal::ContextWindow(window) => self.publish_context_window(window).await,
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
            Outcome::Completed => {
                self.run_events()
                    .publish_agent_event(&self.task_id, AgentSubType::Completed, json!({}))
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
fn interrupted_turn_payload() -> Value {
    json!({
        "message": "Turn interrupted — the sandbox pod was shut down mid-turn. \
                    Resuming when it reschedules.",
        "reason": "sandbox_shutdown",
    })
}

/// Extract a [`WakeupJob`] from a canonical Source block when it is a
/// `ScheduleWakeup` tool_use.
fn extract_wakeup_from_block(block: &CanonicalContentBlock) -> Option<WakeupJob> {
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
        ErrorClass::UnavailableSession => "unavailable_session",
        ErrorClass::InvalidConfiguration => "invalid_configuration",
        ErrorClass::InvalidInput => "invalid_input",
        ErrorClass::NativeProtocol => "native_protocol",
        ErrorClass::ProcessExit => "process_exit",
        ErrorClass::UnresponsiveProcess => "unresponsive_process",
        ErrorClass::Internal => "internal",
    }
}

// ── CoordinatorService ───────────────────────────────────────────────────────

/// Drives a [`Coordinator`] from an [`AgentCommand`] channel.
///
/// Owns the [`Coordinator`], shared services, and the service-lifetime cancel
/// token. The command channel carries [`AgentCommand`]s directly (same type as
/// `AgentHandle` sends) so `AgentHandle`'s public API is unchanged.
pub(crate) struct CoordinatorService {
    coordinator: Coordinator,
    agent_state: Arc<AgentState>,
    config: Arc<Config>,
    credentials: Arc<RwLock<HashMap<String, String>>>,
    backend_client: Arc<dyn BackendClient>,
    system_prompt_cache: Arc<RwLock<Option<String>>>,
    /// Cancelled when `AgentCommand::Stop` is received; also the root for
    /// per-turn child tokens stored in `AgentState`.
    run_cancel: CancellationToken,

    // ── Retry bookkeeping ────────────────────────────────────────────────────
    /// The text submitted with the most recent `AgentCommand::Send`, for
    /// rebuilding the `Input` on a retry.
    last_text: Option<String>,
    /// The model submitted with the most recent `AgentCommand::Send`.
    last_model: Option<String>,
    /// The agent mode submitted with the most recent `AgentCommand::Send`.
    last_agent_mode: Option<String>,
    /// The resume session id submitted with the most recent `AgentCommand::Send`.
    last_resume_session_id: Option<String>,
    /// The `start_fresh` flag from the most recent `AgentCommand::Send`.
    last_start_fresh: bool,
    /// Number of the pending retry attempt (0-indexed; 0 = no retry pending).
    retry_attempt: u32,
    /// When the pending retry should fire. `None` = no retry scheduled. A
    /// `tokio::time::Instant` so the service `select!` can `sleep_until` it.
    retry_at: Option<tokio::time::Instant>,

    // ── Circuit breaker ──────────────────────────────────────────────────────
    /// Consecutive terminal failures without a successful idle transition.
    consecutive_errors: u32,

    // ── Credential recovery ──────────────────────────────────────────────────
    /// Shared detector the harness flips when a Turn fails on a stale credential
    /// (e.g. Codex's `refresh_token_reused`). The service polls it at each
    /// terminal failure to decide whether to pull fresh credentials and re-drive.
    credential_failures: CredentialFailureDetector,
    /// Applies recovered credential values to their sinks (files, env).
    credential_dispatcher: Arc<Dispatcher>,
    /// Credential-recovery re-drives spent on the in-flight prompt. Reset on each
    /// new `Send`/`Recover`; caps at [`MAX_CREDENTIAL_RECOVERY_RETRIES`] so a
    /// permanently invalid credential can't loop forever.
    credential_recovery_attempts: u32,

    // ── Scheduled jobs ───────────────────────────────────────────────────────
    /// Registry the self-scheduling jobs (ScheduleWakeup, background-completion
    /// watch) register into. Their injector re-enters this service's command
    /// channel via the pod's `AgentHandle`.
    scheduled_jobs: Arc<ScheduledJobRegistry>,
    /// The latest `ScheduleWakeup` tool_use seen in the in-flight Turn's Source
    /// blocks, armed at turn end (latest-wins). Subprocess mode only.
    pending_wakeup: Option<WakeupJob>,

    // ── Stop ─────────────────────────────────────────────────────────────────
    /// True while a Stop is in flight — guards the single `Stopped` publish
    /// against a duplicate Stop or a concurrent self-cancel.
    stopping: bool,
    /// The gRPC ack to resolve once the in-flight stop has published its single
    /// terminal event. `None` when the stop carried no ack (shutdown path).
    pending_stop_ack: Option<oneshot::Sender<()>>,
    /// Who initiated the in-flight stop, echoed on the terminal `Stopped`
    /// event. `None` when no `AgentCommand::Stop` set one — a shutdown-initiated
    /// self-cancel, which closes the Turn as `turn_interrupted` (resumable)
    /// rather than `stopped` (terminal). `AgentCommand::Stop` is the only writer,
    /// so `None` at a `Cancelled` outcome *is* the shutdown signature.
    pending_stop_source: Option<StopSource>,
    /// Whether a harness execution is currently in flight (set on
    /// `ExecutionStarted`, cleared on the terminal outcome). Lets the Stop
    /// handler pick the active vs. idle path without an async `AgentState` read.
    turn_active: bool,
}

impl CoordinatorService {
    #[allow(clippy::too_many_arguments)] // reason: service wiring; every dep is a distinct Arc-shared collaborator
    pub(crate) fn new(
        coordinator: Coordinator,
        agent_state: Arc<AgentState>,
        config: Arc<Config>,
        credentials: Arc<RwLock<HashMap<String, String>>>,
        backend_client: Arc<dyn BackendClient>,
        system_prompt_cache: Arc<RwLock<Option<String>>>,
        scheduled_jobs: Arc<ScheduledJobRegistry>,
        credential_failures: CredentialFailureDetector,
        credential_dispatcher: Arc<Dispatcher>,
    ) -> Self {
        Self {
            coordinator,
            agent_state,
            config,
            credentials,
            backend_client,
            system_prompt_cache,
            run_cancel: CancellationToken::new(),
            last_text: None,
            last_model: None,
            last_agent_mode: None,
            last_resume_session_id: None,
            last_start_fresh: false,
            retry_attempt: 0,
            retry_at: None,
            consecutive_errors: 0,
            credential_failures,
            credential_dispatcher,
            credential_recovery_attempts: 0,
            scheduled_jobs,
            pending_wakeup: None,
            stopping: false,
            pending_stop_ack: None,
            pending_stop_source: None,
            turn_active: false,
        }
    }

    /// Drive the service loop: heartbeat ticks, harness signals, and incoming
    /// commands all run inside a single `select!`.
    ///
    /// Returns when `Stop` is received or the harness signal stream closes.
    #[expect(
        clippy::cognitive_complexity,
        reason = "Coordinator event loop still dispatches Agent harness signals and commands until Run and Turn transition policy is extracted"
    )]
    pub(crate) async fn run(
        mut self,
        mut command_rx: mpsc::UnboundedReceiver<AgentCommand>,
        heartbeat: Heartbeat,
    ) {
        let task_id = self.config.task_id.clone();
        info!(task_id = %task_id, "CoordinatorService started");

        // Per-turn cancellation token: refreshed on each ExecutionStarted so
        // an external `agent_state.stop()` cancels the active harness turn.
        let mut turn_cancel = self.run_cancel.child_token();

        let mut heartbeat_tick = tokio::time::interval(HEARTBEAT_TICK_INTERVAL);
        heartbeat_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // A deadline one year in the future — used as the "never fires" sentinel
        // for the retry arm when no retry is scheduled.
        // reason: the larger-unit constructors (`Duration::from_days`) are still
        // unstable, so build the ~1-year sentinel from seconds.
        #[allow(clippy::duration_suboptimal_units)]
        let far_future = tokio::time::Instant::now() + Duration::from_secs(31_536_000);

        loop {
            // Prove the loop is alive on every iteration.
            heartbeat.beat();

            tokio::select! {
                _ = heartbeat_tick.tick() => {
                    // Periodic tick keeps the heartbeat fresh while the loop is
                    // parked on `next_signal` during a long tool call.
                }

                maybe_signal = self.coordinator.next_signal() => {
                    if let Some(signal) = maybe_signal {
                        // Extract turn-cancel side-effects before handing
                        // ownership to handle_signal.
                        let new_session = if let Signal::ExecutionStarted { ref session, .. } = signal {
                            Some(session.clone())
                        } else {
                            None
                        };

                        let outcome_ref = match &signal {
                            Signal::ExecutionFinished { outcome } => Some(outcome.clone()),
                            _ => None,
                        };

                        let rejection_ref = match &signal {
                            Signal::InputRejected { input, reason } => {
                                Some((input.clone(), reason.clone()))
                            }
                            _ => None,
                        };

                        // Observe `ScheduleWakeup` tool_use in Source blocks and
                        // buffer it (latest-wins), to be armed at turn end. In
                        // tmux mode the CLI owns its own wakeup timer, so only
                        // subprocess mode buffers one here.
                        if !self.config.use_tmux {
                            if let Signal::Source(ref event) = signal {
                                if let Some(job) = extract_wakeup_from_block(&event.block) {
                                    self.pending_wakeup = Some(job);
                                }
                            }
                        }

                        // Decide retry BEFORE the coordinator publishes: a
                        // failure we will re-drive must suppress the coordinator's
                        // terminal `error` event so the user sees only
                        // `retrying`.
                        //
                        // A flagged credential takes precedence over transient
                        // retry — a Codex auth failure can surface as a process
                        // exit — and drives credential recovery here, which owns
                        // the retry scheduling, the single reconnect error, and
                        // the consecutive-error accounting for this failure.
                        let mut credential_recovery_handled = false;
                        if let Some(Outcome::Failed(err)) = &outcome_ref {
                            let flagged = self.credential_failures.take_all();
                            if !flagged.is_empty() {
                                // Recovery either schedules a retry (only
                                // `retrying`) or publishes its own single
                                // user-facing reconnect error — the coordinator's
                                // generic terminal error is suppressed either way.
                                self.coordinator.suppress_next_failure_event();
                                self.try_credential_recovery(flagged, err, &task_id).await;
                                credential_recovery_handled = true;
                            } else if is_retryable_harness_error(err)
                                && self.retry_attempt < MAX_RETRIES
                            {
                                self.coordinator.suppress_next_failure_event();
                            }
                        }

                        // Let the coordinator update Run identity, acks,
                        // and publish backend events.
                        self.coordinator.handle_signal(signal).await;

                        // --- Increment 4: AgentState transitions ----------

                        if let Some(session) = new_session {
                            // Mint a fresh per-turn cancel token (child of
                            // run_cancel) so `agent_state.stop()` → token
                            // cancel → the turn_cancel arm below → harness
                            // cancel.
                            turn_cancel = self.run_cancel.child_token();
                            self.agent_state.set_session_id(session.0.clone()).await;
                            self.agent_state
                                .set_run_id(self.coordinator.current_run_id().to_string())
                                .await;
                            self.agent_state
                                .set_running(turn_cancel.clone())
                                .await;
                            self.turn_active = true;
                        }

                        if let Some(outcome) = outcome_ref {
                            // The execution reached its terminal signal.
                            self.turn_active = false;
                            match outcome {
                                Outcome::Completed => {
                                    // Successful completion resets retry and breaker state.
                                    self.retry_attempt = 0;
                                    self.retry_at = None;
                                    self.consecutive_errors = 0;
                                    self.credential_recovery_attempts = 0;
                                    self.agent_state.set_idle().await;
                                    // Arm any self-scheduled jobs the completed Turn requested.
                                    self.arm_scheduled_jobs(&task_id).await;
                                    workspace_sync::upload_after_turn(
                                        &self.config,
                                        self.backend_client.as_ref(),
                                    )
                                    .await;
                                }
                                Outcome::Cancelled => {
                                    self.retry_attempt = 0;
                                    self.retry_at = None;
                                    self.consecutive_errors = 0;
                                    // A cancelled Turn arms nothing; drop any buffered wakeup.
                                    self.pending_wakeup = None;
                                    // The cancellation source owns Turn closure, and
                                    // it also decides *whether the Turn is closed at
                                    // all*. An `AgentCommand::Stop` carries a source
                                    // and is a real stop: it closes the Run and Turn
                                    // with exactly one `Stopped`. A shutdown-initiated
                                    // `AgentState::stop()` carries none — the pod is
                                    // being torn down under a Turn nobody asked to
                                    // end — so it publishes `turn_interrupted`
                                    // instead, leaving the Turn open for the
                                    // replacement pod's `recover_interrupted_run` to
                                    // resume. See `interrupted_turn_payload`.
                                    self.agent_state.set_stopped().await;
                                    let run_events = self
                                        .coordinator
                                        .events
                                        .for_run(self.coordinator.current_run_id());
                                    match self.pending_stop_source.take() {
                                        Some(source) => {
                                            run_events
                                                .publish_agent_event(
                                                    &task_id,
                                                    AgentSubType::Stopped,
                                                    json!({"source": source.as_str()}),
                                                )
                                                .await;
                                        }
                                        None => {
                                            run_events
                                                .publish_task_turn_event(
                                                    &task_id,
                                                    TaskTurnSubType::Interrupted,
                                                    interrupted_turn_payload(),
                                                )
                                                .await;
                                        }
                                    }
                                    // Resolve a waiting gRPC stop only after the
                                    // terminal event is enqueued.
                                    if let Some(ack) = self.pending_stop_ack.take() {
                                        let _ = ack.send(());
                                    }
                                    self.stopping = false;
                                    workspace_sync::upload_after_turn(
                                        &self.config,
                                        self.backend_client.as_ref(),
                                    )
                                    .await;
                                }
                                Outcome::Failed(err) => {
                                    // A failed Turn arms no wakeup; a retry re-emits
                                    // the block if it recurs.
                                    self.pending_wakeup = None;
                                    if credential_recovery_handled {
                                        // Credential recovery (before publish) already
                                        // owns this failure's retry scheduling, the
                                        // single reconnect error, and the
                                        // consecutive-error accounting.
                                    } else if is_retryable_harness_error(&err)
                                        && self.retry_attempt < MAX_RETRIES
                                    {
                                        let attempt = self.retry_attempt;
                                        self.retry_attempt = attempt + 1;

                                        // Record last_error but undo the +1 that
                                        // set_error adds to AgentState's own
                                        // consecutive-error count (read by the
                                        // dispatcher) — a retryable error must not
                                        // count until retries are exhausted (mirrors
                                        // actor.rs:462). The service's own breaker
                                        // counter is only incremented for terminal
                                        // failures below, so it is left untouched here.
                                        self.agent_state.set_error(err.diagnostic.clone()).await;
                                        self.agent_state.undo_error_increment().await;

                                        let backoff = compute_backoff(attempt);
                                        info!(
                                            attempt,
                                            max_retries = MAX_RETRIES,
                                            backoff_secs = backoff.as_secs_f64(),
                                            "Scheduling retry for transient error"
                                        );
                                        self.retry_at = Some(tokio::time::Instant::now() + backoff);

                                        self.coordinator
                                            .events
                                            .for_run(self.coordinator.current_run_id())
                                            .publish_agent_event(
                                                &task_id,
                                                AgentSubType::Retrying,
                                                json!({
                                                    "attempt": attempt,
                                                    "max_retries": MAX_RETRIES,
                                                    "backoff_secs": backoff.as_secs_f64(),
                                                }),
                                            )
                                            .await;
                                    } else {
                                        // Non-retryable or retries exhausted: terminal error.
                                        self.retry_at = None;
                                        self.retry_attempt = 0;
                                        self.consecutive_errors += 1;
                                        self.agent_state.set_error(err.diagnostic.clone()).await;
                                        workspace_sync::upload_after_turn(
                                            &self.config,
                                            self.backend_client.as_ref(),
                                        )
                                        .await;
                                    }
                                }
                            }
                        }

                        if let Some((_, reason)) = rejection_ref {
                            match reason {
                                // A lost-resume rejection the coordinator recovered
                                // (re-drove a fresh Turn, holding the Run) is not a
                                // terminal error — its recovery `ExecutionStarted`
                                // will `set_running` shortly. Only a rejection the
                                // coordinator did NOT recover is a real failure.
                                Rejection::Failed(err) if !self.coordinator.is_recovering() => {
                                    // Pre-accept startup failures also count toward the
                                    // circuit breaker (mirrors actor session-start failure path).
                                    self.consecutive_errors += 1;
                                    self.agent_state.set_error(err.diagnostic.clone()).await;
                                }
                                Rejection::Failed(_) => {}
                                Rejection::Cancelled => {
                                    self.agent_state.set_idle().await;
                                }
                            }
                        }
                    } else {
                        // Harness signal stream closed: the harness has stopped.
                        info!(task_id = %task_id, "Harness signal stream closed; CoordinatorService exiting");
                        break;
                    }
                }

                // When the active turn's cancel token fires (set via
                // `agent_state.set_running(turn_cancel)`), forward the
                // cancellation to the harness so the active execution is
                // interrupted. The select arm stays active as long as the loop
                // runs — once turn_cancel is a cancelled child the arm fires
                // immediately; a new ExecutionStarted mints a fresh child so
                // only the current turn is affected.
                () = turn_cancel.cancelled() => {
                    self.coordinator.cancel();
                    // Re-create so a subsequent turn gets its own cancel scope.
                    turn_cancel = self.run_cancel.child_token();
                }

                // Retry timer: fires when a scheduled retry is due, never fires
                // when no retry is pending (sleep_until(far_future)).
                () = tokio::time::sleep_until(self.retry_at.unwrap_or(far_future)) => {
                    if self.retry_at.take().is_some() {
                        info!(attempt = self.retry_attempt, "Retry timer fired, re-driving turn");

                        // Tell the coordinator to keep the current Run id so the
                        // retried attempt's events join the same Turn.
                        self.coordinator.reuse_run_on_next_start();

                        let text = self.last_text.clone().unwrap_or_default();
                        let model = self.last_model.clone();
                        let agent_mode = self.last_agent_mode.clone();
                        let resume_session_id = self.last_resume_session_id.clone();
                        let start_fresh = self.last_start_fresh;

                        match self.build_input(text, model, agent_mode, resume_session_id, start_fresh).await {
                            Ok(input) => {
                                let (throwaway_tx, _throwaway_rx) = oneshot::channel();
                                if self.coordinator.submit(input, throwaway_tx).await.is_err() {
                                    // HarnessClosed — signal loop will see None next.
                                }
                            }
                            Err(err) => {
                                warn!(error = %err, "Failed to build retry input; aborting retry");
                                self.retry_attempt = 0;
                            }
                        }
                    }
                }

                cmd = command_rx.recv() => {
                    match cmd {
                        Some(AgentCommand::Send {
                            text,
                            model,
                            agent_mode,
                            resume_session_id,
                            start_fresh,
                            ack,
                        }) => {
                            // A fresh user Send gets a fresh credential-recovery
                            // budget: the per-prompt cap is scoped to one prompt.
                            self.credential_recovery_attempts = 0;

                            // ── Circuit breaker ───────────────────────────────────────
                            const MAX_CONSECUTIVE_ERRORS: u32 = 3;
                            if self.consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                                warn!(
                                    consecutive_errors = self.consecutive_errors,
                                    "Circuit breaker: rejecting send"
                                );
                                drop(ack);
                                self.coordinator
                                    .events
                                    .publish_sandbox_event(
                                        &task_id,
                                        SandboxStatus::Error,
                                        json!({
                                            "error": format!(
                                                "Agent failed {} consecutive times. Last error: {}",
                                                self.consecutive_errors,
                                                self.agent_state.last_error().await.unwrap_or_default()
                                            )
                                        }),
                                    )
                                    .await;
                                // Reset so the next user-initiated send gets a fresh budget.
                                self.consecutive_errors = 0;
                                continue;
                            }

                            // Capture last-Send fields for retry rebuilding before
                            // moving text into build_input.
                            self.last_text = Some(text.clone());
                            if model.is_some() {
                                self.last_model = model.clone();
                            }
                            self.last_agent_mode = agent_mode.clone();
                            self.last_resume_session_id = resume_session_id.clone();
                            self.last_start_fresh = start_fresh;

                            let input = match self.build_input(
                                text,
                                self.last_model.clone(),
                                agent_mode,
                                resume_session_id,
                                start_fresh,
                            ).await {
                                Ok(input) => input,
                                Err(err) => {
                                    warn!(error = %err, "Failed to build input; dropping ack");
                                    drop(ack);
                                    continue;
                                }
                            };
                            if self.coordinator.submit(input, ack).await.is_err() {
                                // HarnessClosed — the signal loop will see None next.
                            }
                        }
                        Some(AgentCommand::Recover {
                            text,
                            agent_mode,
                            resume_session_id,
                        }) => {
                            // A replayed run also gets a fresh recovery budget.
                            self.credential_recovery_attempts = 0;

                            // Session state (Claude transcripts + Codex rollouts)
                            // was restored to disk during init before the pod
                            // reached Ready, so the `--resume` this drives sees
                            // the interrupted run's history.
                            let (throwaway_tx, _throwaway_rx) = oneshot::channel();
                            let input = match self.build_input(
                                text,
                                None,
                                agent_mode,
                                Some(resume_session_id),
                                false,
                            ).await {
                                Ok(input) => input,
                                Err(err) => {
                                    warn!(error = %err, "Failed to build recovery input; skipping");
                                    continue;
                                }
                            };
                            if self.coordinator.submit(input, throwaway_tx).await.is_err() {
                                // HarnessClosed.
                            }
                        }
                        Some(AgentCommand::Stop { source, ack }) => {
                            if self.stopping {
                                // A Stop is already in flight; the terminal
                                // `Cancelled` will publish the single `Stopped`.
                                // Drop this ack rather than double-publish.
                                drop(ack);
                                continue;
                            }
                            self.stopping = true;
                            self.pending_stop_ack = ack;
                            self.pending_stop_source = Some(source);
                            self.pending_wakeup = None;
                            self.scheduled_jobs.cancel_all().await;

                            if self.turn_active {
                                // Active stop: interrupt the harness and keep
                                // driving signals. The terminal `Cancelled`
                                // (handled above) publishes the one `Stopped` and
                                // resolves the ack. Do NOT break — the pod stays
                                // alive to serve later turns.
                                info!(task_id = %task_id, "CoordinatorService: active stop");
                                self.coordinator.cancel();
                            } else {
                                // Idle stop: no execution in flight, so no
                                // `Cancelled` will arrive — publish the single
                                // `Stopped` directly and resolve the ack.
                                info!(task_id = %task_id, "CoordinatorService: idle stop");
                                self.agent_state.set_stopped().await;
                                let source = self
                                    .pending_stop_source
                                    .take()
                                    .unwrap_or(StopSource::System);
                                self.coordinator
                                    .events
                                    .for_run(self.coordinator.current_run_id())
                                    .publish_agent_event(
                                        &task_id,
                                        AgentSubType::Stopped,
                                        json!({"source": source.as_str()}),
                                    )
                                    .await;
                                if let Some(ack) = self.pending_stop_ack.take() {
                                    let _ = ack.send(());
                                }
                                self.stopping = false;
                            }
                        }
                        None => {
                            // The command channel closed: the `AgentHandle` was
                            // dropped (process teardown). Exit so the loop ends and
                            // `/livez` observes the closed channel.
                            info!(task_id = %task_id, "CoordinatorService command channel closed");
                            self.run_cancel.cancel();
                            break;
                        }
                    }
                }
            }
        }

        info!(task_id = %task_id, "CoordinatorService exited");
    }

    /// Arm the self-scheduled jobs a just-completed Turn requested: a buffered
    /// `ScheduleWakeup` (latest-wins) and a background-completion watch when
    /// long-running processes outlived the Turn. Both register against
    /// `run_cancel`, so a `Stop` cancels them. Background watches are only armed
    /// in subprocess mode.
    async fn arm_scheduled_jobs(&mut self, task_id: &str) {
        let run_id = self.coordinator.current_run_id().to_string();

        // Wakeup: the latest `ScheduleWakeup` in the Turn replaces any prior one.
        if let Some(job) = self.pending_wakeup.take() {
            let reason = job.reason().to_string();
            info!(reason = %reason, wakeup_at = ?job.wakeup_at(), "Scheduling wakeup");
            self.scheduled_jobs.cancel_by_kind(JobKind::Wakeup).await;
            let registered = self
                .scheduled_jobs
                .register(Box::new(job), self.run_cancel.clone())
                .await;
            if registered {
                self.coordinator
                    .events
                    .for_run(&run_id)
                    .publish_agent_event(
                        task_id,
                        AgentSubType::Scheduled,
                        json!({ "reason": reason }),
                    )
                    .await;
            }
        }

        // Background-completion watch: subprocess mode only, and never while a
        // retry is pending (the Turn is not truly done). `arm` reads `/proc`, so
        // it runs on a blocking thread.
        if !self.config.use_tmux
            && self.config.background_job_monitoring_enabled
            && self.retry_at.is_none()
        {
            let sandbox_pid = std::process::id();
            let armed = tokio::task::spawn_blocking(move || BackgroundWatchJob::arm(sandbox_pid))
                .await
                .ok()
                .flatten();
            if let Some(job) = armed {
                let count = job.process_count();
                self.scheduled_jobs
                    .cancel_by_kind(JobKind::BackgroundWatch)
                    .await;
                let registered = self
                    .scheduled_jobs
                    .register(Box::new(job), self.run_cancel.clone())
                    .await;
                if registered {
                    info!(count, "Armed background-completion watch after turn end");
                    self.coordinator
                        .events
                        .for_run(&run_id)
                        .publish_agent_event(
                            task_id,
                            AgentSubType::Scheduled,
                            json!({ "reason": "background_watch", "processCount": count }),
                        )
                        .await;
                }
            }
        }
    }

    /// Drive credential recovery after a Turn failed with the detector flagged.
    /// Pull fresh values for `flagged`; if any is genuinely fresher than what the
    /// failed Turn used, schedule the same prompt on the same Run for retry (the
    /// caller has already suppressed the coordinator's terminal error, so the
    /// user sees only `retrying`). Otherwise — nothing fresher, no prompt to
    /// retry, or the per-prompt cap exhausted — publish exactly one user-facing
    /// reconnect error and count one consecutive failure. No futile retry.
    ///
    /// The coordinator suppresses the original terminal event, so recovery owns
    /// the single user-facing terminal event.
    async fn try_credential_recovery(
        &mut self,
        flagged: Vec<String>,
        err: &HarnessError,
        task_id: &str,
    ) {
        let cap_reached = self.credential_recovery_attempts >= MAX_CREDENTIAL_RECOVERY_RETRIES;

        // A cap-reached or prompt-less failure can never be re-driven, so skip the
        // (side-effecting) backend pull and go straight to the terminal error.
        let recovered = if cap_reached || self.last_text.is_none() {
            Vec::new()
        } else {
            try_recover_credentials(
                self.backend_client.as_ref(),
                &self.credential_dispatcher,
                task_id,
                &flagged,
                &self.config,
            )
            .await
        };

        let run_id = self.coordinator.current_run_id().to_string();

        if recovered.is_empty() {
            // Nothing fresher (or nothing to retry): surface the one reconnect
            // error and count this failure once.
            let message = credential_reconnect_message(&flagged);
            self.consecutive_errors += 1;
            self.agent_state.set_error(message.clone()).await;
            warn!(
                ?flagged,
                cap_reached, "Credential recovery gave up; surfacing reconnect error"
            );
            self.coordinator
                .events
                .for_run(&run_id)
                .publish_agent_event(
                    task_id,
                    AgentSubType::Error,
                    json!({
                        "error": message,
                        "provider": provider_hint(&flagged),
                        "credential_recovery_exhausted": true,
                        "reason": if cap_reached {
                            "credential_recovery_exhausted"
                        } else {
                            "no_fresher_credential_available"
                        },
                    }),
                )
                .await;
            return;
        }

        // Fresher credential: re-drive the same prompt on the same Run.
        self.credential_recovery_attempts += 1;
        let backoff = compute_backoff(0);
        info!(
            ?recovered,
            attempt = self.credential_recovery_attempts,
            backoff_secs = backoff.as_secs_f64(),
            "Scheduling credential-recovery retry"
        );
        self.retry_at = Some(tokio::time::Instant::now() + backoff);
        // Record the diagnostic for observability, but a scheduled retry must not
        // count toward the breaker (mirrors the transient-retry path).
        self.agent_state.set_error(err.diagnostic.clone()).await;
        self.agent_state.undo_error_increment().await;
        self.coordinator
            .events
            .for_run(&run_id)
            .publish_agent_event(
                task_id,
                AgentSubType::Retrying,
                json!({
                    "attempt": self.credential_recovery_attempts,
                    "max_retries": MAX_CREDENTIAL_RECOVERY_RETRIES,
                    "backoff_secs": backoff.as_secs_f64(),
                    "reason": "credential_recovery",
                    "recovered_keys": recovered,
                }),
            )
            .await;
    }

    /// Build a harness [`Input`] for a single Turn, resolving credentials,
    /// system prompt, and resume from per-Turn sources — mirroring
    /// `session::spawn_session`.
    async fn build_input(
        &self,
        text: String,
        model: Option<String>,
        agent_mode: Option<String>,
        resume_session_id: Option<String>,
        start_fresh: bool,
    ) -> Result<Input, String> {
        // Resolve system prompt from backend, falling back to the cached prompt
        // when a later fetch fails.
        let system_prompt = fetch_system_prompt(
            self.backend_client.as_ref(),
            &self.config.task_id,
            &self.system_prompt_cache,
        )
        .await;

        // Refresh credentials; on failure keep the cached copy (same as
        // `session::spawn_session`).
        match self
            .backend_client
            .get_credentials(&self.config.task_id)
            .await
        {
            Ok(resp) => {
                crate::init::credentials::persist_agent_credentials(
                    &self.config,
                    &resp.credentials,
                    crate::init::credentials::PersistMode::Refresh,
                )
                .await;
                *self.credentials.write().await = filter_agent_credentials(&resp.credentials);
            }
            Err(error) => {
                warn!(error = %error, "Failed to refresh credentials, using cached");
            }
        }

        let creds = self.credentials.read().await;
        let env: Option<HashMap<String, String>> = if creds.is_empty() {
            None
        } else {
            Some(creds.clone())
        };
        drop(creds);

        // Resolve resume exactly as `session::resolve_resume_session`: the pod's
        // last known Session id takes precedence, then the backend-provided id,
        // then a `--continue` for the Claude family. The harness is fixed for the
        // Sandbox, so the family is `config.agent_type`, not the per-Turn value.
        let harness_family = self
            .config
            .agent_type
            .as_deref()
            .and_then(|t| resolve_agent_harness(t).ok())
            .unwrap_or(AGENT_TYPE_CLAUDE_CODE);
        let resume = build_resume(
            self.agent_state
                .last_session_id()
                .await
                .as_deref()
                .unwrap_or(""),
            resume_session_id.as_deref(),
            harness_family,
            start_fresh,
        );

        Ok(Input {
            id: InputId(uuid::Uuid::new_v4().to_string()),
            text,
            model,
            mode: agent_mode,
            resume,
            start_fresh,
            env,
            system_prompt,
        })
    }
}

/// Fetch the assembled append-system-prompt from the backend. On failure,
/// fall back to whatever was last cached in this pod. Cache is purely a
/// resilience layer: every successful fetch refreshes it.
async fn fetch_system_prompt(
    backend_client: &dyn BackendClient,
    task_id: &str,
    system_prompt_cache: &RwLock<Option<String>>,
) -> Option<String> {
    match backend_client.get_system_prompt(task_id).await {
        Ok(prompt) => {
            *system_prompt_cache.write().await = Some(prompt.clone());
            Some(prompt)
        }
        Err(error) => {
            let cached = system_prompt_cache.read().await.clone();
            warn!(
                error = %error,
                has_cache = cached.is_some(),
                "Failed to fetch system prompt; using cached fallback"
            );
            cached
        }
    }
}

/// Resolve the `ResumeSession` for a new harness `Input`. Mirrors
/// `session::resolve_resume_session`: the pod's last known Session id wins, then
/// the backend-provided id, then a `--continue` (`ResumeSession::Last`) for the
/// Claude family (Codex has no `--continue`, so it starts fresh).
fn build_resume(
    previous_session_id: &str,
    resume_session_id: Option<&str>,
    agent_type: &str,
    start_fresh: bool,
) -> Option<ResumeSession> {
    if start_fresh {
        return None;
    }
    if !previous_session_id.is_empty() {
        return Some(ResumeSession::WithId(previous_session_id.to_string()));
    }
    if let Some(session_id) = resume_session_id.filter(|s| !s.is_empty()) {
        return Some(ResumeSession::WithId(session_id.to_string()));
    }
    if agent_type == AGENT_TYPE_CLAUDE_CODE {
        Some(ResumeSession::Last)
    } else {
        None
    }
}

/// A human-readable provider name for a set of flagged credential keys, for the
/// user-facing reconnect message.
fn provider_hint(flagged: &[String]) -> &'static str {
    flagged
        .iter()
        .find_map(|k| match k.as_str() {
            "CODEX_AUTH_JSON" => Some("Codex"),
            "CLAUDE_CODE_OAUTH_TOKEN" => Some("Claude"),
            "GITHUB_TOKEN" => Some("GitHub"),
            _ => None,
        })
        .unwrap_or("provider")
}

/// The single user-facing "reconnect" message surfaced when credential recovery
/// cannot re-drive (nothing fresher, or the per-prompt cap is exhausted).
fn credential_reconnect_message(flagged: &[String]) -> String {
    format!(
        "Sign-in expired. Please reconnect {} in Settings.",
        provider_hint(flagged)
    )
}

#[cfg(test)]
mod tests;
