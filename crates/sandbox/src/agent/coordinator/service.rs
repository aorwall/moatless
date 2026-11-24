//! Lifecycle policy over a [`Coordinator`].
//!
//! Retry with backoff, the consecutive-error circuit breaker, wakeup and
//! background-watch scheduling, credential-recovery retry, and `AgentState`
//! transitions. The seam mapping itself is [`super::seam`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use moatless_types::agent::AgentSubType;
use moatless_types::credential_key::USER_AUTH_TOKEN;
use moatless_types::sandbox::SandboxStatus;
use moatless_types::task::TaskTurnSubType;
use serde_json::json;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::seam::{Coordinator, extract_wakeup_from_block, interrupted_turn_payload};
use crate::agent::agent_state::AgentState;
use crate::agent::background_watch_job::BackgroundWatchJob;
use crate::agent::handle::{AgentCommand, RunAck, resolve_agent_harness};
use crate::agent::harness::{
    HarnessError, HarnessRuntime, Input, InputId, Outcome, Rejection, Signal,
};
use crate::agent::heartbeat::{HEARTBEAT_TICK_INTERVAL, Heartbeat};
use crate::agent::retry::{MAX_RETRIES, compute_backoff, is_retryable_harness_error};
use crate::agent::scheduled_job::{JobKind, ScheduledJobRegistry};
use crate::agent::types::{ForkSource, ResumeSession, StopSource};
use crate::agent::wakeup_job::WakeupJob;
use crate::backend_client::BackendClient;
use crate::commands::CommandRegistry;
use crate::config::Config;
use crate::credentials::Dispatcher;
use crate::credentials::fetch::agent_credentials;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::credentials::retry::{MAX_CREDENTIAL_RECOVERY_RETRIES, try_recover_credentials};
use crate::workspace_sync::{self, WorkspaceSyncTracker};

/// Drives a [`Coordinator`] from an [`AgentCommand`] channel.
///
/// Owns the coordinator, shared services, and the service-lifetime cancel
/// token; the channel carries the same type `AgentHandle` sends, so its public
/// API is unchanged.
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
    /// The reasoning effort submitted with the most recent
    /// `AgentCommand::Send`.
    last_effort: Option<String>,
    /// The resume session id submitted with the most recent
    /// `AgentCommand::Send`.
    last_resume_session_id: Option<String>,
    /// The `start_fresh` flag from the most recent `AgentCommand::Send`.
    last_start_fresh: bool,
    /// The fork anchors from the most recent `AgentCommand::Send`.
    ///
    /// Kept so a retried first turn of a fork branches from the same cut,
    /// rather than falling back to an empty session.
    last_fork: Option<ForkSource>,
    /// The `switch_harness` flag from the most recent `AgentCommand::Send`,
    /// kept for the same reason as `last_fork`.
    last_switch_harness: bool,
    /// Builds the harness for another Agent profile; `None` keeps the one
    /// the service was built with.
    harness_factory: Option<HarnessFactory>,
    /// Number of the pending retry attempt (0-indexed; 0 = no retry pending).
    retry_attempt: u32,
    /// When the pending retry should fire. `None` = no retry scheduled. A
    /// `tokio::time::Instant` so the service `select!` can `sleep_until` it.
    retry_at: Option<tokio::time::Instant>,

    // ── Circuit breaker ──────────────────────────────────────────────────────
    /// Consecutive terminal failures without a successful idle transition.
    consecutive_errors: u32,

    // ── Credential recovery ──────────────────────────────────────────────────
    /// Shared detector the harness flips when a Turn fails on a stale
    /// credential (e.g. Codex's `refresh_token_reused`). The service polls it
    /// at each terminal failure to decide whether to pull fresh credentials and
    /// re-drive.
    credential_failures: CredentialFailureDetector,
    /// Applies recovered credential values to their sinks (files, env).
    credential_dispatcher: Arc<Dispatcher>,
    /// Credential-recovery re-drives spent on the in-flight prompt. Reset on
    /// each new `Send`/`Recover`; caps at [`MAX_CREDENTIAL_RECOVERY_RETRIES`]
    /// so a permanently invalid credential can't loop forever.
    credential_recovery_attempts: u32,

    // ── Scheduled jobs ───────────────────────────────────────────────────────
    /// Registry the self-scheduling jobs (`ScheduleWakeup`,
    /// background-completion watch) register into. Their injector re-enters
    /// this service's command channel via the pod's `AgentHandle`.
    scheduled_jobs: Arc<ScheduledJobRegistry>,
    /// Commands this task started through `moat cmd`. The turn-end watch arms
    /// over whichever are still running.
    commands: Arc<CommandRegistry>,
    /// The latest `ScheduleWakeup` `tool_use` seen in the in-flight Turn's
    /// Source blocks, armed at turn end (latest-wins). Subprocess mode only.
    pending_wakeup: Option<WakeupJob>,

    // ── Stop ─────────────────────────────────────────────────────────────────
    /// True while a Stop is in flight — guards the single `Stopped` publish
    /// against a duplicate Stop or a concurrent self-cancel.
    stopping: bool,
    /// The gRPC ack to resolve once the in-flight stop has published its single
    /// terminal event. `None` when the stop carried no ack (shutdown path).
    pending_stop_ack: Option<oneshot::Sender<()>>,
    /// Who initiated the in-flight stop, echoed on the terminal `Stopped`
    /// event.
    ///
    /// `None` means no `AgentCommand::Stop` set one: a shutdown self-cancel
    /// closes the Turn as `turn_interrupted`, not `stopped` — the only
    /// writer, so `None` at `Cancelled` is the shutdown signature.
    pending_stop_source: Option<StopSource>,
    // ── Workspace sync ───────────────────────────────────────────────────────
    /// The pod-wide upload baseline. Turn boundaries are the primary
    /// durability point on the PVC tier; sharing this keeps a turn that
    /// changed nothing from shipping a duplicate tar.
    ///
    /// Cloned into a background task rather than awaited, since this loop
    /// also drains messages, stops, and command wakes.
    workspace_sync: WorkspaceSyncTracker,
}

struct SendCommand {
    text: String,
    model: Option<String>,
    agent_mode: Option<String>,
    effort: Option<String>,
    resume_session_id: Option<String>,
    start_fresh: bool,
    fork: Option<ForkSource>,
    agent_profile: Option<String>,
    switch_harness: bool,
    ack: oneshot::Sender<RunAck>,
}

/// Builds the harness a service runs for an Agent profile, from the service's
/// `Config` with that profile set.
pub(crate) type HarnessFactory =
    Box<dyn Fn(&Arc<Config>, &str) -> Result<HarnessRuntime, HarnessError> + Send + Sync>;

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
        commands: Arc<CommandRegistry>,
        credential_failures: CredentialFailureDetector,
        credential_dispatcher: Arc<Dispatcher>,
        workspace_sync: WorkspaceSyncTracker,
    ) -> Self {
        // The root lives here, but each turn's token is minted where the turn
        // is opened — inside the coordinator, in the same act as the state
        // transition and the publish.
        let run_cancel = CancellationToken::new();
        let mut coordinator = coordinator;
        coordinator.bind(Arc::clone(&agent_state), run_cancel.clone());
        coordinator.agent_profile = config.agent_profile.clone();

        Self {
            coordinator,
            agent_state,
            config,
            credentials,
            backend_client,
            system_prompt_cache,
            run_cancel,
            last_text: None,
            last_model: None,
            last_agent_mode: None,
            last_effort: None,
            last_resume_session_id: None,
            last_start_fresh: false,
            last_fork: None,
            last_switch_harness: false,
            harness_factory: None,
            retry_attempt: 0,
            retry_at: None,
            consecutive_errors: 0,
            credential_failures,
            credential_dispatcher,
            credential_recovery_attempts: 0,
            scheduled_jobs,
            commands,
            pending_wakeup: None,
            stopping: false,
            pending_stop_ack: None,
            pending_stop_source: None,
            workspace_sync,
        }
    }

    /// Rebuild the harness through `factory` when a Send names another Agent
    /// profile.
    #[must_use]
    pub(crate) fn with_harness_factory(mut self, factory: HarnessFactory) -> Self {
        self.harness_factory = Some(factory);
        self
    }

    /// [`Self::run`], once `setup_gate` opens.
    ///
    /// Commands sent meanwhile queue in `command_rx` in order, so a replayed
    /// turn still starts ahead of a message sent after it. The wait beats no
    /// heartbeat: `/livez` reads a stale one as unhealthy only mid-turn.
    pub(crate) async fn run_after(
        self,
        setup_gate: crate::init::SetupGate,
        command_rx: mpsc::Receiver<AgentCommand>,
        heartbeat: Heartbeat,
    ) {
        setup_gate.wait().await;
        self.run(command_rx, heartbeat).await;
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
        mut command_rx: mpsc::Receiver<AgentCommand>,
        heartbeat: Heartbeat,
    ) {
        let task_id = self.config.task_id.clone();
        info!(task_id = %task_id, "CoordinatorService started");

        // Per-turn cancellation token: refreshed on each ExecutionStarted so
        // an external `agent_state.stop()` cancels the active harness turn.
        let mut turn_cancel = self.run_cancel.child_token();

        let mut heartbeat_tick = tokio::time::interval(HEARTBEAT_TICK_INTERVAL);
        heartbeat_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // "Never fires" sentinel for the retry arm when no retry is scheduled.
        // reason: `Duration::from_days` is still unstable, so build it from seconds.
        #[allow(clippy::duration_suboptimal_units)]
        let far_future = tokio::time::Instant::now() + Duration::from_secs(31_536_000);

        loop {
            heartbeat.beat();

            tokio::select! {
                _ = heartbeat_tick.tick() => {
                    // Periodic tick keeps the heartbeat fresh while the loop is
                    // parked on `next_signal` during a long tool call.
                }

                maybe_signal = self.coordinator.next_signal() => {
                    if let Some(signal) = maybe_signal {
                        // Copy out what the service's own bookkeeping needs
                        // before handing ownership to `handle_signal`. The turn
                        // transition is not among them: that travels with the
                        // signal.
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
                            if let Signal::Source(ref block) = signal {
                                if let Some(job) = extract_wakeup_from_block(block) {
                                    self.pending_wakeup = Some(job);
                                }
                            }
                        }

                        // Decide retry before the coordinator publishes: a
                        // re-driven failure must suppress the terminal
                        // `error` event. A flagged credential wins over
                        // transient retry (Codex auth can exit as a process)
                        // and owns the retry, reconnect error, and error count.
                        let mut credential_recovery_handled = false;
                        if let Some(Outcome::Failed(err)) = &outcome_ref {
                            let flagged = self.credential_failures.take_all();
                            if !flagged.is_empty() {
                                // Recovery schedules a retry or publishes its own
                                // reconnect error; the coordinator's terminal
                                // error is suppressed either way.
                                self.coordinator.suppress_next_failure_event();
                                self.try_credential_recovery(flagged, err, &task_id).await;
                                credential_recovery_handled = true;
                            } else if is_retryable_harness_error(err)
                                && self.retry_attempt < MAX_RETRIES
                            {
                                self.coordinator.suppress_next_failure_event();
                                // Record last_error but undo set_error's +1 to
                                // AgentState's own error count (read by the
                                // dispatcher): a retryable error must not count
                                // until retries exhaust. The service's own
                                // breaker counter stays untouched here.
                                self.agent_state.set_error(err.diagnostic.clone()).await;
                                self.agent_state.undo_error_increment().await;
                            } else {
                                // Non-retryable, or retries exhausted: this
                                // failure is terminal.
                                self.consecutive_errors += 1;
                                self.agent_state.set_error(err.diagnostic.clone()).await;
                            }
                        }

                        // Counted before the coordinator publishes `Completed`,
                        // over the same set `arm_scheduled_jobs` then watches.
                        if matches!(outcome_ref, Some(Outcome::Completed { .. }))
                            && !self.config.use_tmux
                        {
                            let owed = self.commands.unreported().await.len();
                            self.coordinator
                                .defer_completion_for_background_commands(owed);
                        }

                        // The coordinator updates Run identity, acks, and
                        // publishes as one act per signal, keeping the events'
                        // turn and the agent's reported turn the same turn.
                        // What's left here is service-owned: retry budget,
                        // breaker, scheduled jobs, workspace sync.
                        if let Some(opened) = self.coordinator.handle_signal(signal).await {
                            turn_cancel = opened;
                        }

                        if let Some(outcome) = outcome_ref {
                            match outcome {
                                Outcome::Completed { .. } => {
                                    self.retry_attempt = 0;
                                    self.retry_at = None;
                                    self.consecutive_errors = 0;
                                    self.credential_recovery_attempts = 0;
                                    self.arm_scheduled_jobs(&task_id).await;
                                    workspace_sync::spawn_after_turn(
                                        self.config.clone(),
                                        self.backend_client.clone(),
                                        self.workspace_sync.clone(),
                                    );
                                }
                                Outcome::Cancelled => {
                                    self.retry_attempt = 0;
                                    self.retry_at = None;
                                    self.consecutive_errors = 0;
                                    // A cancelled Turn arms nothing; drop any buffered wakeup.
                                    self.pending_wakeup = None;
                                    // Source decides closure: `Stop` closes
                                    // Run+Turn with one `Stopped`; a shutdown
                                    // self-cancel (no source) leaves the Turn
                                    // open via `turn_interrupted` to resume.
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
                                    workspace_sync::spawn_after_turn(
                                        self.config.clone(),
                                        self.backend_client.clone(),
                                        self.workspace_sync.clone(),
                                    );
                                }
                                Outcome::Failed(err) => {
                                    // A failed Turn arms no wakeup; a retry re-emits
                                    // the block if it recurs.
                                    self.pending_wakeup = None;
                                    if credential_recovery_handled {
                                        // Credential recovery already owns this
                                        // failure's retry, reconnect error, and
                                        // error accounting (decided before publish).
                                    } else if is_retryable_harness_error(&err)
                                        && self.retry_attempt < MAX_RETRIES
                                    {
                                        let attempt = self.retry_attempt;
                                        self.retry_attempt = attempt + 1;

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
                                        // The status and the breaker count were
                                        // settled before the publish, above.
                                        self.retry_at = None;
                                        self.retry_attempt = 0;
                                        workspace_sync::spawn_after_turn(
                                            self.config.clone(),
                                            self.backend_client.clone(),
                                            self.workspace_sync.clone(),
                                        );
                                    }
                                }
                            }
                        }

                        if let Some((_, reason)) = rejection_ref {
                            match reason {
                                // A lost-resume rejection already recovered
                                // (re-drove a Turn, holding the Run) is not a
                                // terminal error; only an unrecovered
                                // rejection is a real failure.
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
                        info!(task_id = %task_id, "HarnessRuntime signal stream closed; CoordinatorService exiting");
                        break;
                    }
                }

                // When the turn's cancel token fires (set via
                // `agent_state.set_running(turn_cancel)`), forward cancellation
                // to the harness. `turn_cancel` is rebound to a fresh child on
                // each `ExecutionStarted`, so only the current turn is affected.
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
                        let effort = self.last_effort.clone();
                        let resume_session_id = self.last_resume_session_id.clone();
                        let start_fresh = self.last_start_fresh;
                        let fork = self.last_fork.clone();
                        let switch_harness = self.last_switch_harness;

                        match self.build_input(text, model, agent_mode, effort, resume_session_id, start_fresh, fork, switch_harness).await {
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
                            effort,
                            resume_session_id,
                            start_fresh,
                            fork,
                            agent_profile,
                            switch_harness,
                            ack,
                        }) => {
                            let send = SendCommand {
                                text,
                                model,
                                agent_mode,
                                effort,
                                resume_session_id,
                                start_fresh,
                                fork,
                                agent_profile,
                                switch_harness,
                                ack,
                            };
                            self.handle_send_command(&task_id, send).await;
                        }
                        Some(AgentCommand::Recover {
                            text,
                            agent_mode,
                            effort,
                            model,
                            resume_session_id,
                            run_id,
                        }) => {
                            // A replayed run also gets a fresh recovery budget.
                            self.credential_recovery_attempts = 0;

                            // Adopt the interrupted turn's run_id so the backend
                            // maps this re-drive back to that turn. Skip for a
                            // legacy turn with no recorded run_id — it forks a
                            // fresh turn, as before.
                            if let Some(run_id) = run_id.filter(|id| !id.is_empty()) {
                                self.coordinator.resume_run(run_id);
                            }

                            // Session state (Claude transcripts + Codex rollouts)
                            // was restored to disk during init before the pod
                            // reached Ready, so the `--resume` this drives sees
                            // the interrupted run's history.
                            let (throwaway_tx, _throwaway_rx) = oneshot::channel();
                            let input = match self.build_input(
                                text,
                                model,
                                agent_mode,
                                effort,
                                Some(resume_session_id),
                                false,
                                // A recovery replays a run that already had a
                                // Session of its own; the fork it may have
                                // started from is long spent.
                                None,
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

                            if self.coordinator.turn_active() {
                                // Active stop: interrupt the harness, keep
                                // driving signals. The terminal `Cancelled`
                                // above publishes `Stopped` and resolves the
                                // ack — do NOT break; the pod serves later turns.
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
    /// `ScheduleWakeup` (latest-wins) and a background-completion watch for
    /// processes that outlived the Turn.
    ///
    /// Both register against `run_cancel`, so a `Stop` cancels them; background
    /// watches are subprocess-mode only.
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
        // retry is pending (the Turn is not truly done).
        if !self.config.use_tmux && self.retry_at.is_none() {
            let armed = BackgroundWatchJob::arm(Arc::clone(&self.commands)).await;
            if let Some(job) = armed {
                let count = job.command_count();
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

    async fn handle_send_command(&mut self, task_id: &str, send: SendCommand) {
        // A fresh user Send gets a fresh credential-recovery budget: the
        // per-prompt cap is scoped to one prompt.
        self.credential_recovery_attempts = 0;

        const MAX_CONSECUTIVE_ERRORS: u32 = 3;
        if self.consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
            warn!(
                consecutive_errors = self.consecutive_errors,
                "Circuit breaker: rejecting send"
            );
            drop(send.ack);
            self.coordinator
                .events
                .publish_sandbox_event(
                    task_id,
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
            return;
        }

        // A turn in flight keeps its harness: dropping the ack leaves the batch
        // unacked, so the dispatcher redelivers it once the turn ends.
        if !self.ensure_profile(send.agent_profile.as_deref()).await {
            drop(send.ack);
            return;
        }

        // Capture last-Send fields for retry rebuilding before moving text into
        // build_input.
        self.last_text = Some(send.text.clone());
        if send.model.is_some() {
            self.last_model = send.model.clone();
        }
        self.last_agent_mode = send.agent_mode.clone();
        self.last_effort = send.effort.clone();
        self.last_resume_session_id = send.resume_session_id.clone();
        self.last_start_fresh = send.start_fresh;
        self.last_fork = send.fork.clone();
        self.last_switch_harness = send.switch_harness;

        let input = match self
            .build_input(
                send.text,
                self.last_model.clone(),
                send.agent_mode,
                send.effort,
                send.resume_session_id,
                send.start_fresh,
                send.fork,
                send.switch_harness,
            )
            .await
        {
            Ok(input) => input,
            Err(err) => {
                warn!(error = %err, "Failed to build input; dropping ack");
                drop(send.ack);
                return;
            }
        };
        if self.coordinator.submit(input, send.ack).await.is_err() {
            // HarnessClosed — the signal loop will see None next.
        }
    }

    /// Run `agent_profile` from the next turn on, rebuilding the harness when
    /// it differs from the running one. False while a turn is open, which the
    /// harness must finish first; a profile this Sandbox cannot build keeps
    /// the running harness.
    pub(super) async fn ensure_profile(&mut self, agent_profile: Option<&str>) -> bool {
        let Some(profile) = agent_profile.filter(|p| !p.is_empty()) else {
            return true;
        };
        if self.config.agent_profile.as_deref() == Some(profile) {
            return true;
        }
        let Some(factory) = self.harness_factory.as_ref() else {
            return true;
        };
        if self.coordinator.turn_active() {
            info!(
                agent_profile = profile,
                "Agent profile changed mid-turn; redelivering after it"
            );
            return false;
        }
        let mut config = (*self.config).clone();
        config.agent_profile = Some(profile.to_string());
        let config = Arc::new(config);
        let harness = match factory(&config, profile) {
            Ok(harness) => harness,
            Err(error) => {
                warn!(agent_profile = profile, error = %error.diagnostic,
                    "Cannot build the harness for the new Agent profile; keeping the running one");
                return true;
            }
        };
        let family = |c: &Config| {
            c.agent_profile
                .as_deref()
                .and_then(|p| resolve_agent_harness(p).ok())
        };
        // The kept model names the other harness's catalog: a fallback Send
        // carries none, so keeping it sends Claude Code a Codex model.
        if family(&config) != family(&self.config) {
            self.agent_state.clear_session_id().await;
            self.last_model = None;
        }
        info!(
            from = ?self.config.agent_profile,
            to = profile,
            "Rebuilt the harness for the Task's new Agent profile"
        );
        self.coordinator.replace_harness(harness, profile);
        self.config = config;
        true
    }

    /// Drive credential recovery after a Turn failed with the detector flagged.
    ///
    /// Pulls fresh values for `flagged`; if any is fresher, retries the same
    /// prompt on the same Run (`retrying`). Otherwise, past the per-prompt
    /// cap, publishes exactly one reconnect error — recovery owns the single
    /// user-facing event the coordinator suppressed.
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
    #[allow(
        clippy::too_many_arguments,
        reason = "one Turn's inputs; grouping them into a struct only moves the argument list"
    )]
    async fn build_input(
        &self,
        text: String,
        model: Option<String>,
        agent_mode: Option<String>,
        effort: Option<String>,
        resume_session_id: Option<String>,
        start_fresh: bool,
        fork: Option<ForkSource>,
        switch_harness: bool,
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
                crate::credentials::fetch::persist_agent_credentials(
                    &self.config,
                    crate::credentials::fetch::PersistMode::Refresh,
                )
                .await;
                crate::credentials::fetch::apply_git_identity(&resp.credentials).await;
                // Through the sink, so the `moat` CLI and proxied git follow a
                // driver change with the turn rather than the next refresh.
                if let Some(token) = resp.credentials.get(USER_AUTH_TOKEN) {
                    if let Err(error) = self
                        .credential_dispatcher
                        .dispatch(USER_AUTH_TOKEN, token)
                        .await
                    {
                        warn!(%error, "Failed to apply the fetched task token");
                    }
                }
                *self.credentials.write().await =
                    agent_credentials(&resp.credentials, self.config.backend_api_url.as_deref());
            }
            Err(error) => {
                warn!(error = %error, "Failed to refresh credentials, using cached");
            }
        }

        let mut env: HashMap<String, String> = self.credentials.read().await.clone();
        // Stamped per Member; see `crates/sandbox/AGENTS.md` "Spawning a child process".
        env.insert("TASK_ID".to_string(), self.config.task_id.clone());
        let env = Some(env);

        // Resolve resume as `session::resolve_resume_session`: the pod's last
        // known Session id wins, then the backend-provided id, then
        // `--continue` for Claude. Family is `config.agent_profile`, kept
        // current by `ensure_profile`.
        let harness_family = self
            .config
            .agent_profile
            .as_deref()
            .and_then(|t| resolve_agent_harness(t).ok())
            .unwrap_or(moatless_types::agent::Harness::ClaudeCode);
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
            effort,
            resume,
            start_fresh,
            fork,
            switch_harness,
            env,
            system_prompt,
        })
    }
}

/// Fetch the assembled append-system-prompt from the backend. On failure, fall
/// back to whatever was last cached in this pod. Cache is purely a resilience
/// layer: every successful fetch refreshes it.
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

/// Resolve the `ResumeSession` for a new harness `Input`, mirroring
/// `session::resolve_resume_session`.
///
/// Precedence: the pod's last known Session id, then the backend-provided id,
/// then `--continue` for the Claude family (Codex has none, so it starts
/// fresh).
pub(super) fn build_resume(
    previous_session_id: &str,
    resume_session_id: Option<&str>,
    harness: moatless_types::agent::Harness,
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
    harness
        .resumes_last_session()
        .then_some(ResumeSession::Last)
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
