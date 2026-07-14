//! The agent actor event loop.
//!
//! Runs a `tokio::select!` that reads runtime messages from the active session
//! and processes incoming commands. Manages the full session lifecycle.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use claude_agent_sdk::types::messages::ResultMessage;
use claude_agent_sdk::{SdkError, SdkMessage};
use moatless_events::{AgentSubType, SandboxSubType};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agent::agent_state::AgentState;
use crate::agent::background_watch_job::BackgroundWatchJob;
use crate::agent::codex_exec::CodexExecEvent;
use crate::agent::heartbeat::{Heartbeat, HEARTBEAT_TICK_INTERVAL};
use crate::agent::scheduled_job::{JobKind, ScheduledJobRegistry};
use crate::agent::wakeup_job::WakeupJob;
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::credentials::Dispatcher;
use crate::events::GrpcEventPublisher;

use super::claude::process_claude_message;
use super::codex::{handle_codex_stream_error, process_codex_message};
use super::credential_recovery::try_credential_recovery_retry;
use super::retry::{
    compute_backoff, is_retryable_result, is_retryable_runtime_error, RetryState, MAX_RETRIES,
};
use super::session::{
    fetch_system_prompt, normalize_agent_type, publish_turn_start, start_session, TmuxSessionInfo,
};
use super::{drain_pending_acks, ActiveHandle, AgentCommand, QueuedSend};

#[allow(clippy::large_enum_variant)] // Claude SDK messages are much larger than Codex JSON events.
enum ActiveMessage {
    Claude(Result<SdkMessage, SdkError>),
    Codex(Result<CodexExecEvent, String>),
}

/// Mutable loop state + shared services handed to the message processors.
///
/// Constructed inside a `select!` branch body immediately before calling a
/// processor; NLL ends the borrows right after the call so the rest of the
/// branch keeps using the bare locals.
pub(super) struct TurnContext<'a> {
    pub(super) agent_state: &'a AgentState,
    pub(super) events: &'a GrpcEventPublisher,
    pub(super) task_id: &'a str,
    pub(super) session_id: &'a mut String,
    pub(super) handle: &'a mut Option<ActiveHandle>,
    pub(super) pending_acks: &'a mut VecDeque<oneshot::Sender<String>>,
    pub(super) consecutive_errors: &'a mut u32,
    /// Claude-only, untouched by Codex.
    pub(super) pending_wakeup: &'a mut Option<WakeupJob>,
    /// Claude-only.
    pub(super) last_stop_reason: &'a mut Option<String>,
    /// Claude/tmux-only.
    pub(super) tmux_turn_in_flight: &'a mut bool,
    /// Codex-only.
    pub(super) spawn_nonce: &'a mut String,
    /// Codex-only.
    pub(super) codex_turn_output: &'a mut String,
    /// Claude-only: when the last rate-limited `agent.activity` event was
    /// published (see the `ToolProgress` arm in `process_claude_message`).
    pub(super) last_activity_publish: &'a mut Option<std::time::Instant>,
}

/// The actor event loop.
///
/// Runs a `tokio::select!` that reads runtime messages from the active session
/// and processes incoming commands. Manages the full session lifecycle.
#[allow(clippy::too_many_arguments)] // reason: actor wiring; refactor pending
pub(super) async fn agent_loop(
    mut command_rx: mpsc::UnboundedReceiver<AgentCommand>,
    config: Arc<Config>,
    agent_state: Arc<AgentState>,
    events: GrpcEventPublisher,
    credentials: Arc<RwLock<HashMap<String, String>>>,
    backend_client: Arc<dyn BackendClient>,
    scheduled_jobs: Arc<ScheduledJobRegistry>,
    credential_failures: CredentialFailureDetector,
    credential_dispatcher: Arc<Dispatcher>,
    system_prompt_cache: Arc<RwLock<Option<String>>>,
    heartbeat: Heartbeat,
) {
    let task_id = config.task_id.clone();
    let mut handle: Option<ActiveHandle> = None;
    let mut session_id = String::new();
    // Per-spawn nonce included in every UUID emitted from the active codex
    // spawn. Required because Codex CLI restarts its `item_id` numbering at
    // `item_0` on every `codex exec --resume`, which would otherwise collide
    // with prior turns under the stable `session_id`. A random nonce (rather
    // than a counter) survives pod restarts: counter-based reset to 0 would
    // re-collide once thread state is reloaded from the workspace PVC.
    let mut spawn_nonce = String::new();
    let mut consecutive_errors: u32 = 0;
    let mut pending_acks: VecDeque<oneshot::Sender<String>> = VecDeque::new();
    let mut queued_sends: VecDeque<QueuedSend> = VecDeque::new();
    let mut codex_turn_output = String::new();
    let mut last_model: Option<String> = None;
    let mut last_agent_type: Option<String> = None;
    let mut last_agent_mode: Option<String> = None;
    let mut retry_state: Option<RetryState> = None;
    let mut last_prompt: Option<String> = None;
    let mut pending_wakeup: Option<WakeupJob> = None;
    // tmux persistent-session state. In tmux mode the Claude handle survives
    // across turns (the TUI keeps running so the CLI's internal ScheduleWakeup
    // timer and Monitor watcher stay alive), so the loop tracks whether a turn
    // is currently in flight and the fingerprint of the live session.
    let mut tmux_turn_in_flight = false;
    let mut tmux_session: Option<TmuxSessionInfo> = None;
    // Stop reason of the most recent assistant message in the current turn. The
    // CLI `result` message carries no stop_reason, so we capture it here and inject
    // it into the agent/completed payload (lets the UI tell `end_turn` from
    // `max_tokens` / `tool_use` without digging into per-message rows).
    let mut last_stop_reason: Option<String> = None;
    // When the last rate-limited `agent.activity` liveness event was published
    // (advances the reaper's idle floor during long tool calls).
    let mut last_activity_publish: Option<std::time::Instant> = None;
    // Tracks per-prompt credential-recovery retries so a stuck-loop credential
    // can't drive infinite re-tries.
    let mut credential_recovery_attempts: u32 = 0;
    // True while a Codex turn is in flight: set once a Codex session starts
    // (after `taskTurn/active` is published) and cleared by any terminal Codex
    // path (`TurnCompleted`/`TurnFailed`/`Error`, stream error, stop). Codex
    // runs one turn per process, so if the handle is cleared while this is still
    // set the turn ended without a terminal event — the loop reports that as
    // `codex_incomplete` rather than letting it pass in silence.
    let mut codex_turn_active = false;
    // Agent-lifetime cancel token: survives individual sessions, cancelled on Stop.
    let agent_run_cancel = CancellationToken::new();

    // Liveness heartbeat: bumped every iteration below and on a periodic tick so
    // the `/livez` probe can tell a wedged actor from a slow-but-alive turn.
    let mut heartbeat_tick = tokio::time::interval(HEARTBEAT_TICK_INTERVAL);
    heartbeat_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    info!(task_id = %task_id, "Agent actor started");

    loop {
        // Prove the actor loop is alive on every iteration. The periodic
        // `heartbeat_tick` branch below keeps this fresh even while the loop is
        // parked waiting for runtime messages during a long tool call.
        heartbeat.beat();

        // Turn-level invariant for Codex: a cleared handle with no terminal
        // event published means the turn ended silently. Surface it as an
        // explicit error so the backend can close the turn. Normal terminal
        // paths clear `codex_turn_active` in the same step they clear the
        // handle, so this never fires on a clean completion.
        if codex_turn_active && handle.is_none() && retry_state.is_none() {
            warn!(
                task_id = %task_id,
                "Codex turn handle cleared without a terminal event; reporting incomplete"
            );
            codex_turn_active = false;
            let error_text = "Codex agent stopped without reporting a result \
                (turn ended without a terminal event).";
            agent_state.set_error(error_text.to_string()).await;
            events
                .publish_agent_event(
                    &task_id,
                    AgentSubType::Error,
                    serde_json::json!({
                        "error": error_text,
                        "reason": "codex_incomplete",
                    }),
                )
                .await;
        }

        // tmux persistent mode: the handle stays alive between turns, so an
        // idle live session (no turn in flight) is also "ready" for the next
        // queued send — pasted into the running TUI instead of respawning.
        let tmux_idle_session = config.use_tmux
            && !tmux_turn_in_flight
            && matches!(handle, Some(ActiveHandle::Claude(_)));

        if (handle.is_none() || tmux_idle_session) && retry_state.is_none() {
            if let Some(send) = queued_sends.pop_front() {
                if tmux_idle_session {
                    // Restart the session when the send is incompatible with
                    // the live session's fingerprint, otherwise paste the
                    // message into the running TUI as a new turn.
                    let fresh_system_prompt =
                        fetch_system_prompt(&*backend_client, &task_id, &system_prompt_cache)
                            .await;
                    let restart = tmux_session.as_ref().is_none_or(|info| {
                        info.needs_restart(&send, fresh_system_prompt.as_deref(), &session_id)
                    });
                    if restart {
                        info!("Send incompatible with live tmux session; restarting session");
                        if let Some(active) = handle.take() {
                            active.close();
                        }
                        tmux_session = None;
                        queued_sends.push_front(send);
                        // Next iteration takes the handle-is-none path below
                        // and respawns with `--resume`.
                        continue;
                    }
                    last_prompt = Some(send.text.clone());
                    let Some(active) = handle.as_ref() else {
                        // Unreachable: `tmux_idle_session` implies a live handle.
                        queued_sends.push_front(send);
                        continue;
                    };
                    match active.send(&send.text, &session_id).await {
                        Ok(()) => {
                            info!("Pasted new turn into live tmux session");
                            tmux_turn_in_flight = true;
                            let turn_cancel = tmux_session.as_ref().map_or_else(
                                || agent_run_cancel.clone(),
                                |info| info.cancel_token.clone(),
                            );
                            agent_state.set_running(turn_cancel).await;
                            publish_turn_start(&events, &task_id).await;
                            let _ = send.ack.send(session_id.clone());
                        }
                        Err(error) => {
                            warn!(
                                error = %error,
                                "Failed to paste into live tmux session; restarting session"
                            );
                            if let Some(active) = handle.take() {
                                active.close();
                            }
                            tmux_session = None;
                            queued_sends.push_front(send);
                        }
                    }
                    continue;
                }

                last_prompt = Some(send.text.clone());
                pending_acks.push_back(send.ack);

                match start_session(
                    &send.text,
                    &config,
                    Arc::clone(&agent_state),
                    &events,
                    &credentials,
                    &*backend_client,
                    &task_id,
                    &session_id,
                    send.model.as_deref(),
                    send.agent_type.as_deref(),
                    send.agent_mode.as_deref(),
                    send.resume_session_id.clone(),
                    credential_failures.clone(),
                    Arc::clone(&system_prompt_cache),
                )
                .await
                {
                    Ok(started) => {
                        codex_turn_active = matches!(started.handle, ActiveHandle::Codex(_));
                        if config.use_tmux && matches!(started.handle, ActiveHandle::Claude(_)) {
                            tmux_turn_in_flight = true;
                            tmux_session = Some(TmuxSessionInfo {
                                model: send.model.clone(),
                                agent_type: normalize_agent_type(send.agent_type.as_deref()),
                                agent_mode: send.agent_mode.clone(),
                                system_prompt: started.system_prompt.clone(),
                                cancel_token: started.cancel_token.clone(),
                            });
                        }
                        handle = Some(started.handle);
                    }
                    Err(error) => {
                        warn!(error = %error, "Failed to start agent session");
                        drain_pending_acks(&mut pending_acks);

                        if is_retryable_runtime_error(&error) && MAX_RETRIES > 0 {
                            let backoff = compute_backoff(0);
                            info!(
                                backoff_secs = backoff.as_secs_f64(),
                                "Scheduling retry after session start failure"
                            );
                            retry_state = Some(RetryState {
                                prompt: send.text,
                                model: send.model,
                                agent_type: send.agent_type,
                                agent_mode: send.agent_mode,
                                attempt: 0,
                                retry_at: Instant::now() + backoff,
                            });
                            events
                                .publish_agent_event(
                                    &task_id,
                                    AgentSubType::Retrying,
                                    serde_json::json!({
                                        "attempt": 0,
                                        "max_retries": MAX_RETRIES,
                                        "backoff_secs": backoff.as_secs_f64(),
                                    }),
                                )
                                .await;
                        } else {
                            consecutive_errors += 1;
                            agent_state.set_error(error.to_string()).await;
                            events
                                .publish_agent_event(
                                    &task_id,
                                    AgentSubType::Error,
                                    serde_json::json!({ "error": error.to_string() }),
                                )
                                .await;
                        }
                    }
                }

                continue;
            }
        }

        tokio::select! {
            _ = heartbeat_tick.tick() => {
                // Wake periodically so the heartbeat stays fresh while the loop
                // is otherwise parked on `messages.recv()` (model thinking / a
                // long-running tool call emits no messages). Falls through to the
                // top-of-loop `beat()` on the next iteration.
            }

            msg = async {
                match handle.as_mut() {
                    Some(ActiveHandle::Claude(handle)) => {
                        handle.messages.recv().await.map(ActiveMessage::Claude)
                    }
                    Some(ActiveHandle::Codex(handle)) => {
                        handle.messages.recv().await.map(ActiveMessage::Codex)
                    }
                    None => std::future::pending::<Option<ActiveMessage>>().await,
                }
            } => {
                match msg {
                    Some(ActiveMessage::Claude(Ok(sdk_msg))) => {
                        let retry_candidate = if let SdkMessage::Result(ref result) = sdk_msg {
                            Some(result.clone())
                        } else {
                            None
                        };

                        let turn_cancel = tmux_session.as_ref().map_or_else(
                            || agent_run_cancel.clone(),
                            |info| info.cancel_token.clone(),
                        );
                        let mut ctx = TurnContext {
                            agent_state: &agent_state,
                            events: &events,
                            task_id: &task_id,
                            session_id: &mut session_id,
                            handle: &mut handle,
                            pending_acks: &mut pending_acks,
                            consecutive_errors: &mut consecutive_errors,
                            pending_wakeup: &mut pending_wakeup,
                            last_stop_reason: &mut last_stop_reason,
                            tmux_turn_in_flight: &mut tmux_turn_in_flight,
                            spawn_nonce: &mut spawn_nonce,
                            codex_turn_output: &mut codex_turn_output,
                            last_activity_publish: &mut last_activity_publish,
                        };
                        process_claude_message(
                            sdk_msg,
                            &mut ctx,
                            config.use_tmux,
                            &turn_cancel,
                        ).await;

                        // Session ended (subprocess turn end or an error
                        // result closing the persistent tmux session) — drop
                        // the stale tmux fingerprint.
                        if handle.is_none() {
                            tmux_session = None;
                            tmux_turn_in_flight = false;
                        }

                        if let Some(ref result) = retry_candidate {
                            if is_retryable_result(result) {
                                let attempt = retry_state.as_ref().map_or(0, |state| state.attempt + 1);
                                if attempt < MAX_RETRIES {
                                    if let Some(ref prompt) = last_prompt {
                                        agent_state.undo_error_increment().await;
                                        consecutive_errors = consecutive_errors.saturating_sub(1);

                                        let backoff = compute_backoff(attempt);
                                        info!(
                                            attempt,
                                            max_retries = MAX_RETRIES,
                                            backoff_secs = backoff.as_secs_f64(),
                                            "Scheduling retry for transient error"
                                        );

                                        retry_state = Some(RetryState {
                                            prompt: prompt.clone(),
                                            model: last_model.clone(),
                                            agent_type: last_agent_type.clone(),
                                            agent_mode: last_agent_mode.clone(),
                                            attempt,
                                            retry_at: Instant::now() + backoff,
                                        });

                                        events.publish_agent_event(
                                            &task_id,
                                            AgentSubType::Retrying,
                                            serde_json::json!({
                                                "attempt": attempt,
                                                "max_retries": MAX_RETRIES,
                                                "backoff_secs": backoff.as_secs_f64(),
                                            }),
                                        ).await;
                                    }
                                } else {
                                    info!("Max retries ({MAX_RETRIES}) exhausted");
                                    retry_state = None;
                                }
                            } else {
                                retry_state = None;
                            }
                        }

                        // Arm the ScheduleWakeup timer if one was queued and the
                        // turn completed without error. Latest-wins: any prior
                        // pending wakeup is cancelled before the new one is
                        // registered. Subprocess mode only: in tmux mode the
                        // CLI's internal timer owns wakeups (the persistent TUI
                        // survives turn ends) and `pending_wakeup` is never set.
                        if let Some(job) = pending_wakeup.take() {
                            let is_error = retry_candidate
                                .as_ref()
                                .is_some_and(|r| matches!(r, ResultMessage::Error(_)) || matches!(r, ResultMessage::Success(s) if s.is_error));

                            if is_error {
                                debug!("Ignoring ScheduleWakeup because turn ended with error");
                            } else {
                                let reason = job.reason().to_string();
                                info!(
                                    reason = %reason,
                                    wakeup_at = ?job.wakeup_at(),
                                    "Scheduling wakeup"
                                );
                                scheduled_jobs.cancel_by_kind(JobKind::Wakeup).await;
                                let registered = scheduled_jobs
                                    .register(Box::new(job), agent_run_cancel.clone())
                                    .await;
                                if registered {
                                    events.publish_agent_event(
                                        &task_id,
                                        AgentSubType::Scheduled,
                                        serde_json::json!({ "reason": reason }),
                                    ).await;
                                }
                            }
                        }

                        // Subprocess mode: a clean turn end can leave a detached
                        // background process (e.g. `setsid`/`nohup … &`) still
                        // executing. CLI-managed `run_in_background` shells are
                        // disabled in this mode (see build_options) because the
                        // one-shot `claude -p` process kills them at turn end;
                        // detached processes survive but the exited CLI can't
                        // emit their completion notification, so arm a
                        // sandbox-local watch that re-triggers the agent once
                        // those processes finish. Latest-wins: re-arming each
                        // turn end refreshes the watched PID set. Skipped when a
                        // retry is queued (the agent resumes on its own) or the
                        // turn errored.
                        if let Some(ref result) = retry_candidate {
                            let turn_failed = matches!(result, ResultMessage::Error(_))
                                || matches!(result, ResultMessage::Success(s) if s.is_error);
                            if !config.use_tmux
                                && handle.is_none()
                                && retry_state.is_none()
                                && !turn_failed
                            {
                                let sandbox_pid = std::process::id();
                                let armed = tokio::task::spawn_blocking(move || {
                                    BackgroundWatchJob::arm(sandbox_pid)
                                })
                                .await
                                .ok()
                                .flatten();
                                if let Some(job) = armed {
                                    let count = job.process_count();
                                    scheduled_jobs
                                        .cancel_by_kind(JobKind::BackgroundWatch)
                                        .await;
                                    let registered = scheduled_jobs
                                        .register(Box::new(job), agent_run_cancel.clone())
                                        .await;
                                    if registered {
                                        info!(
                                            count,
                                            "Armed background-completion watch after turn end"
                                        );
                                        events.publish_agent_event(
                                            &task_id,
                                            AgentSubType::Scheduled,
                                            serde_json::json!({
                                                "reason": "background_watch",
                                                "processCount": count,
                                            }),
                                        ).await;
                                    }
                                }
                            }
                        }
                    }
                    Some(ActiveMessage::Claude(Err(error))) => {
                        if matches!(error, claude_agent_sdk::SdkError::Cancelled) {
                            info!("Claude session cancelled");
                            handle = None;
                            tmux_session = None;
                            tmux_turn_in_flight = false;
                        } else if config.use_tmux && !tmux_turn_in_flight {
                            // The persistent tmux session died between turns
                            // (no turn in flight) — not a turn failure. Clear
                            // the handle; the next send respawns the session
                            // with `--resume`.
                            warn!(error = %error, "Persistent tmux session ended while idle");
                            handle = None;
                            tmux_session = None;
                        } else {
                            warn!(error = %error, "Claude SDK message error");
                            // A non-cancel stream error ends the turn. Surface it
                            // explicitly and clear the handle so the subsequent
                            // transport-close (`None`) arm doesn't re-report it.
                            handle = None;
                            tmux_session = None;
                            tmux_turn_in_flight = false;
                            agent_state.set_error(error.to_string()).await;
                            events
                                .publish_agent_event(
                                    &task_id,
                                    AgentSubType::Error,
                                    serde_json::json!({
                                        "error": error.to_string(),
                                        "reason": "stream_error",
                                    }),
                                )
                                .await;
                        }
                    }
                    Some(ActiveMessage::Codex(Ok(event))) => {
                        let was_terminal_error = matches!(
                            event,
                            CodexExecEvent::TurnFailed(_) | CodexExecEvent::Error(_)
                        );
                        // Any of the three terminal events publishes a terminal
                        // agent event and clears the handle, ending the turn.
                        let was_terminal = was_terminal_error
                            || matches!(event, CodexExecEvent::TurnCompleted(_));
                        let mut ctx = TurnContext {
                            agent_state: &agent_state,
                            events: &events,
                            task_id: &task_id,
                            session_id: &mut session_id,
                            handle: &mut handle,
                            pending_acks: &mut pending_acks,
                            consecutive_errors: &mut consecutive_errors,
                            pending_wakeup: &mut pending_wakeup,
                            last_stop_reason: &mut last_stop_reason,
                            tmux_turn_in_flight: &mut tmux_turn_in_flight,
                            spawn_nonce: &mut spawn_nonce,
                            codex_turn_output: &mut codex_turn_output,
                            last_activity_publish: &mut last_activity_publish,
                        };
                        process_codex_message(event, last_model.as_deref(), &mut ctx).await;
                        if was_terminal {
                            codex_turn_active = false;
                        }
                        retry_state = None;
                        if was_terminal_error {
                            try_credential_recovery_retry(
                                &credential_failures,
                                &credential_dispatcher,
                                &*backend_client,
                                &config,
                                &task_id,
                                &last_prompt,
                                &last_model,
                                &last_agent_type,
                                &last_agent_mode,
                                &mut retry_state,
                                &mut credential_recovery_attempts,
                                &mut consecutive_errors,
                                &agent_state,
                                &events,
                            ).await;
                        }
                    }
                    Some(ActiveMessage::Codex(Err(error))) => {
                        let mut ctx = TurnContext {
                            agent_state: &agent_state,
                            events: &events,
                            task_id: &task_id,
                            session_id: &mut session_id,
                            handle: &mut handle,
                            pending_acks: &mut pending_acks,
                            consecutive_errors: &mut consecutive_errors,
                            pending_wakeup: &mut pending_wakeup,
                            last_stop_reason: &mut last_stop_reason,
                            tmux_turn_in_flight: &mut tmux_turn_in_flight,
                            spawn_nonce: &mut spawn_nonce,
                            codex_turn_output: &mut codex_turn_output,
                            last_activity_publish: &mut last_activity_publish,
                        };
                        handle_codex_stream_error(&error, last_model.as_deref(), &mut ctx).await;
                        codex_turn_active = false;
                        retry_state = None;
                        try_credential_recovery_retry(
                            &credential_failures,
                            &credential_dispatcher,
                            &*backend_client,
                            &config,
                            &task_id,
                            &last_prompt,
                            &last_model,
                            &last_agent_type,
                            &last_agent_mode,
                            &mut retry_state,
                            &mut credential_recovery_attempts,
                            &mut consecutive_errors,
                            &agent_state,
                            &events,
                        ).await;
                    }
                    None => {
                        debug!("Session transport closed");
                        if config.use_tmux && !tmux_turn_in_flight && handle.is_some() {
                            // The idle persistent tmux session's transport
                            // closed with no turn in flight — not an abnormal
                            // turn end. Clear the handle; the next send
                            // respawns the session with `--resume`.
                            info!("Persistent tmux session transport closed while idle");
                            handle = None;
                            tmux_session = None;
                        } else if handle.is_some() {
                            // The subprocess transport closed while a turn was still
                            // active — it exited WITHOUT emitting a `result` message
                            // (crash, SIGKILL, OOM, context overflow). A normal
                            // completion sets `handle = None` when the Result is
                            // processed (see `process_claude_message`), and a cancel
                            // clears it in the `Err(Cancelled)` arm, so reaching here
                            // with `handle.is_some()` is always an abnormal, otherwise
                            // -silent stop. Publish an explicit error so the backend
                            // raises a taskTurn/error instead of the agent going quiet.
                            warn!(
                                task_id = %task_id,
                                "Agent transport closed without a result; turn ended unexpectedly"
                            );
                            handle = None;
                            tmux_session = None;
                            tmux_turn_in_flight = false;
                            // This arm itself publishes the terminal event, so the
                            // top-of-loop incomplete guard must not double-report.
                            codex_turn_active = false;
                            let error_text = "Agent process exited unexpectedly without \
                                completing the turn (no result received — likely a crash, \
                                out-of-memory, or terminated subprocess).";
                            agent_state.set_error(error_text.to_string()).await;
                            events
                                .publish_agent_event(
                                    &task_id,
                                    AgentSubType::Error,
                                    serde_json::json!({
                                        "error": error_text,
                                        "reason": "transport_closed",
                                    }),
                                )
                                .await;
                        }
                    }
                }
            }

            () = async {
                match retry_state.as_ref() {
                    Some(state) => tokio::time::sleep_until(state.retry_at).await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(state) = retry_state.take() else {
                    continue;
                };

                info!(attempt = state.attempt, "Retry timer fired, starting new session");

                match start_session(
                    &state.prompt,
                    &config,
                    Arc::clone(&agent_state),
                    &events,
                    &credentials,
                    &*backend_client,
                    &task_id,
                    &session_id,
                    state.model.as_deref(),
                    state.agent_type.as_deref(),
                    state.agent_mode.as_deref(),
                    None,
                    credential_failures.clone(),
                    Arc::clone(&system_prompt_cache),
                ).await {
                    Ok(started) => {
                        codex_turn_active = matches!(started.handle, ActiveHandle::Codex(_));
                        if config.use_tmux && matches!(started.handle, ActiveHandle::Claude(_)) {
                            tmux_turn_in_flight = true;
                            tmux_session = Some(TmuxSessionInfo {
                                model: state.model.clone(),
                                agent_type: normalize_agent_type(state.agent_type.as_deref()),
                                agent_mode: state.agent_mode.clone(),
                                system_prompt: started.system_prompt.clone(),
                                cancel_token: started.cancel_token.clone(),
                            });
                        }
                        handle = Some(started.handle);
                    }
                    Err(error) => {
                        warn!(error = %error, attempt = state.attempt, "Retry session start failed");
                        if is_retryable_runtime_error(&error) && state.attempt + 1 < MAX_RETRIES {
                            let next_attempt = state.attempt + 1;
                            let backoff = compute_backoff(next_attempt);
                            info!(
                                attempt = next_attempt,
                                backoff_secs = backoff.as_secs_f64(),
                                "Scheduling next retry after session start failure"
                            );
                            retry_state = Some(RetryState {
                                prompt: state.prompt,
                                model: state.model,
                                agent_type: state.agent_type,
                                agent_mode: state.agent_mode,
                                attempt: next_attempt,
                                retry_at: Instant::now() + backoff,
                            });
                            events.publish_agent_event(
                                &task_id,
                                AgentSubType::Retrying,
                                serde_json::json!({
                                    "attempt": next_attempt,
                                    "max_retries": MAX_RETRIES,
                                    "backoff_secs": backoff.as_secs_f64(),
                                }),
                            ).await;
                        } else {
                            consecutive_errors += 1;
                            agent_state.set_error(error.to_string()).await;
                            events.publish_agent_event(
                                &task_id,
                                AgentSubType::Error,
                                serde_json::json!({ "error": error.to_string() }),
                            ).await;
                        }
                    }
                }
            }

            cmd = command_rx.recv() => {
                match cmd {
                    Some(AgentCommand::Send { text, model, agent_type, agent_mode, resume_session_id, ack }) => {
                        if retry_state.is_some() {
                            info!("New send command cancels pending retry");
                            retry_state = None;
                        }
                        credential_recovery_attempts = 0;

                        if model.is_some() {
                            last_model = model.clone();
                        }
                        if agent_type.is_some() {
                            last_agent_type = agent_type.clone();
                        }
                        if agent_mode.is_some() {
                            last_agent_mode = agent_mode.clone();
                        }

                        const MAX_CONSECUTIVE_ERRORS: u32 = 3;
                        if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                            warn!(
                                consecutive_errors,
                                "Circuit breaker: {consecutive_errors} consecutive errors, rejecting send"
                            );
                            drop(ack);
                            events.publish_sandbox_event(
                                &task_id,
                                SandboxSubType::Error,
                                serde_json::json!({
                                    "error": format!(
                                        "Agent failed {consecutive_errors} consecutive times. Last error: {}",
                                        agent_state.last_error().await.unwrap_or_default()
                                    )
                                }),
                            ).await;
                            // Reset so the next user-initiated send gets a fresh budget.
                            // Otherwise once tripped, no further sends ever reach the agent
                            // — leaving no path for `consecutive_errors` to drop back to 0.
                            consecutive_errors = 0;
                            continue;
                        }

                        let effective_send = QueuedSend {
                            text: text.clone(),
                            model: last_model.clone(),
                            agent_type: last_agent_type.clone(),
                            agent_mode: last_agent_mode.clone(),
                            resume_session_id,
                            ack,
                        };
                        last_prompt = Some(text);

                        match handle.as_ref() {
                            Some(active_handle @ ActiveHandle::Claude(_)) => {
                                if config.use_tmux && !tmux_turn_in_flight {
                                    // Idle persistent tmux session: queue and
                                    // let the drain path decide paste-vs-restart
                                    // against the session fingerprint.
                                    queued_sends.push_back(effective_send);
                                } else if let Err(error) = active_handle.send(&effective_send.text, &session_id).await {
                                    warn!(error = %error, "Failed to inject message into Claude session");
                                    drop(effective_send.ack);
                                } else {
                                    info!("Injected message into running Claude session");
                                    let _ = effective_send.ack.send(session_id.clone());
                                }
                            }
                            Some(ActiveHandle::Codex(_)) => {
                                info!("Queueing message until active Codex turn completes");
                                queued_sends.push_back(effective_send);
                            }
                            None => {
                                queued_sends.push_back(effective_send);
                            }
                        }
                    }
                    Some(AgentCommand::Recover { text, agent_type, agent_mode, resume_session_id }) => {
                        if retry_state.is_some() {
                            info!("Recover command cancels pending retry");
                            retry_state = None;
                        }
                        credential_recovery_attempts = 0;

                        if agent_type.is_some() {
                            last_agent_type = agent_type.clone();
                        }
                        if agent_mode.is_some() {
                            last_agent_mode = agent_mode.clone();
                        }

                        let (recover_ack_tx, _recover_ack_rx) = oneshot::channel();
                        let queued = QueuedSend {
                            text: text.clone(),
                            model: last_model.clone(),
                            agent_type: last_agent_type.clone(),
                            agent_mode: last_agent_mode.clone(),
                            resume_session_id: Some(resume_session_id),
                            ack: recover_ack_tx,
                        };
                        last_prompt = Some(text);
                        queued_sends.push_back(queued);
                    }
                    Some(AgentCommand::Stop) | None => {
                        info!("Agent actor stopping");
                        drop(retry_state.take());
                        drop(pending_wakeup.take());
                        while let Some(send) = queued_sends.pop_front() {
                            drop(send.ack);
                        }
                        drain_pending_acks(&mut pending_acks);
                        if let Some(active_handle) = handle.take() {
                            active_handle.close();
                        }
                        scheduled_jobs.cancel_all().await;
                        agent_run_cancel.cancel();
                        agent_state.set_stopped().await;
                        events.publish_agent_event(
                            &task_id,
                            AgentSubType::Stopped,
                            serde_json::json!({}),
                        ).await;
                        break;
                    }
                }
            }
        }
    }

    info!(task_id = %task_id, "Agent actor exited");
}
