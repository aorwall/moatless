use super::{CodexConfig, classify};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agent::codex_app_server;
use crate::agent::codex_app_server::AppServerSession;
use crate::agent::codex_app_server::ServerEvent;
use crate::agent::codex_app_server::protocol::{
    ItemNotification, ThreadItem, TokenUsageBreakdown, TurnCompletedNotification, TurnError,
    TurnStatus,
};
use crate::agent::types::{AgentRunRequest, ResumeSession};
use crate::credentials::recovery::CODEX_AUTH_JSON;

use crate::agent::harness::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use crate::agent::harness::process::{AgentProcess, ProcessEvent, ProcessTermination};
use crate::agent::harness::signal::{ContextWindow, Input, InputId, SessionId};

use super::normalize::{Normalizer, TurnUsage};

/// How long a mid-Turn message waits for the steerability gate before it is
/// handed back for the next Turn instead.
///
/// A healthy Turn never reaches it: `turn/started` follows `turn/start`'s
/// response as fast as the server can write the line. It bounds the one wait in
/// `submit` that depends on a notification arriving at all — a server that never
/// sent `turn/started` would otherwise hold the message for the life of the
/// Turn. On expiry the message opens the next Turn, the same fallback a rejected
/// steer takes.
///
/// It deliberately does not cover the `turn/steer` that follows the gate, which
/// must not be abandoned once written: see [`AgentProcess::submit`].
const MID_TURN_DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Where the Turn is in its life, which is what decides whether a message that
/// arrives now can join it.
///
/// The gate between the first two states is measured, not defensive: steering
/// with the correct turn id immediately after `turn/start`'s **response** is
/// rejected with `-32600 "no active turn to steer"`, and the Turn only becomes
/// steerable when the `turn/started` **notification** arrives (see the spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnPhase {
    /// `turn/start` has been sent and `turn/started` has not arrived. Not
    /// steerable, and token-usage readings here are the Session's replayed
    /// total rather than anything this Turn spent.
    Requested,
    /// `turn/started` arrived: the Turn is running and takes `turn/steer`.
    Running,
    /// `turn/completed` (or a stream failure) ended it. Nothing will make it
    /// steerable again.
    Ended,
}

/// One Codex app-server subprocess for a single Turn.
pub(crate) struct CodexProcess {
    cfg: CodexConfig,
    session: Option<AppServerSession>,
    /// Pre-emitted events (a `SessionReady` is always paired with an
    /// `InputAccepted`) waiting to be drained one at a time by [`Self::next`].
    pending: VecDeque<ProcessEvent>,
    input_id: Option<InputId>,
    session_ready: bool,
    input_accepted: bool,
    normalizer: Normalizer,
    /// The Codex thread. Needed by every `turn/*` request; the exec transport
    /// only ever forwarded the Session id outward.
    thread_id: Option<String>,
    /// The Turn this process is running, from `turn/start` and confirmed by the
    /// `turn/started` notification. `turn/steer` and `turn/interrupt` need it.
    turn_id: Option<String>,
    /// How far the Turn has got, which is what the steerability gate reads.
    phase: TurnPhase,
    /// The message this Turn is being asked to take, while it waits for the gate.
    ///
    /// One slot, because there can only ever be one: the driver submits inputs
    /// one at a time and `submit` empties this again before it answers, so a
    /// second arrival cannot begin until the first has been dealt with.
    held: Option<Input>,
    /// What the Session had already spent when this Turn opened. `Some` only on
    /// a resumed thread, whose stored total is replayed before the Turn runs.
    usage_baseline: Option<TokenUsageBreakdown>,
    /// The most recent cumulative reading, held because usage streams before
    /// the terminal instead of riding on it.
    usage_latest: Option<TokenUsageBreakdown>,
    /// Tears the server down, and exists before there is a server to tear down.
    /// The session is handed this same token, so a Stop that arrives while the
    /// Turn is still opening kills the child mid-handshake instead of waiting
    /// for a reply that may never come.
    cancel_token: CancellationToken,
}

impl CodexProcess {
    pub(crate) fn new(cfg: CodexConfig) -> Self {
        Self {
            cfg,
            session: None,
            pending: VecDeque::new(),
            input_id: None,
            session_ready: false,
            input_accepted: false,
            normalizer: Normalizer::new(None),
            thread_id: None,
            turn_id: None,
            phase: TurnPhase::Requested,
            held: None,
            usage_baseline: None,
            usage_latest: None,
            cancel_token: CancellationToken::new(),
        }
    }

    /// Build a process backed by an already-open (injected) session for
    /// deterministic driver-level lifecycle tests. `submit` then records the
    /// input against this session instead of spawning a real CLI.
    #[cfg(test)]
    pub(crate) fn with_session(cfg: CodexConfig, session: AppServerSession, model: &str) -> Self {
        let mut process = Self::new(cfg);
        process.session = Some(session);
        process.normalizer = Normalizer::new(Some(model.to_string()));
        process
    }

    /// Whether the underlying session has been torn down — the observable effect
    /// of [`AgentProcess::cancel`] on the Codex process tree.
    #[cfg(test)]
    pub(crate) fn session_cancelled(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(AppServerSession::is_cancelled)
    }

    /// Announce the Session and the acceptance of the opening input, once.
    ///
    /// Both are known as soon as the thread is open. This is driven from the
    /// `thread/start` / `thread/resume` **response** rather than the
    /// `thread/started` notification, because a resume emits no such
    /// notification (measured) — waiting for one would hang every Turn after the
    /// first.
    fn announce_session(&mut self, thread_id: String) {
        let session = SessionId(thread_id);
        self.normalizer.set_session(session.clone());
        if !self.session_ready {
            self.session_ready = true;
            // `open_turn` set `self.turn_id` from the `turn/start` response before
            // this runs (submit and event reads never overlap), so a real turn's
            // id is present. A driver-injected session has none, and reports None.
            self.pending.push_back(ProcessEvent::SessionReady {
                session,
                agent_turn_id: self.turn_id.clone(),
            });
        }
        if !self.input_accepted {
            self.input_accepted = true;
            if let Some(input) = self.input_id.clone() {
                self.pending
                    .push_back(ProcessEvent::InputAccepted { input });
            }
        }
    }

    /// Open the Turn: spawn the server, start or resume the thread, and run one
    /// Turn on it, returning the thread the Turn is running on. Every step's
    /// failure is classified, and a failed resume is never quietly turned into a
    /// fresh thread.
    async fn open_turn(&mut self, input: &Input) -> Result<String, HarnessError> {
        // `start_fresh` always wins; otherwise the resume request rides straight
        // to the server.
        let resume = if input.start_fresh {
            None
        } else {
            input.resume.clone()
        };

        let request = AgentRunRequest {
            content: Vec::new(),
            model: input.model.clone(),
            system_prompt: input.system_prompt.clone(),
            resume: resume.clone(),
            agent_type: Some("codex".to_string()),
            agent_mode: input.mode.clone(),
            effort: input.effort.clone(),
        };
        let config = Arc::clone(&self.cfg.config);
        let model = codex_app_server::resolve_codex_model(&config, &request);

        let session =
            codex_app_server::start(&config, input.env.clone(), self.cancel_token.clone())
                .await
                .map_err(|error| classify::classify_startup_error(&error))?;

        let thread_id = match (&input.fork, &resume) {
            // A fork's first turn branches a new thread off the parent's at the
            // cut, then runs its prompt on the copy. The parent is untouched.
            (Some(fork), _) => {
                let params = codex_app_server::thread_fork_params(
                    &config,
                    &fork.source_session_id,
                    fork.source_agent_turn_id.as_deref(),
                );
                session
                    .fork_thread(params)
                    .await
                    .map_err(|error| classify::classify_request_error("thread/fork", &error))?
            }
            (None, Some(ResumeSession::WithId(thread_id))) => {
                let params = codex_app_server::thread_resume_params(&config, &request, thread_id);
                session
                    .resume_thread(params)
                    .await
                    .map_err(|error| classify::classify_request_error("thread/resume", &error))?
            }
            // `resume --last` has no app-server equivalent and no caller:
            // `build_resume` only ever yields it for the Claude family. If one
            // ever appears, say so rather than resuming something unintended.
            (None, other) => {
                if other.is_some() {
                    warn!("Codex cannot resume the last thread by name; starting a fresh one");
                }
                let params = codex_app_server::thread_start_params(&config, &request);
                session
                    .start_thread(params)
                    .await
                    .map_err(|error| classify::classify_request_error("thread/start", &error))?
            }
        };

        let params =
            codex_app_server::turn_start_params(&thread_id, &input.text, &config, &request);
        let turn_id = session
            .start_turn(params)
            .await
            .map_err(|error| classify::classify_request_error("turn/start", &error))?;

        info!(%thread_id, %turn_id, %model, "Codex Turn started");
        self.normalizer = Normalizer::new(Some(model));
        self.thread_id = Some(thread_id.clone());
        self.turn_id = Some(turn_id);
        self.session = Some(session);
        Ok(thread_id)
    }

    /// Deliver what [`Self::held`] owes the running Turn.
    ///
    /// A delivered message reports exactly one bare `InputAccepted`, which is
    /// what puts it on the running Turn rather than opening a second execution.
    ///
    /// An error means this Turn cannot take the message, and the driver queues it
    /// for the next one (`driver.rs`). That is the only way back: the driver's
    /// queue is reachable *only* through an error from `submit`, and a message
    /// reported neither accepted nor rejected leaves the coordinator holding its
    /// acknowledgement, which pins the dispatcher's single-flight lock for the
    /// life of the pod. So nothing is answered `Ok` until the server has taken
    /// it, and `submit` clears whatever is still held when it answers with one.
    ///
    /// Only ever called once the gate is open — see [`Self::await_steerable`].
    async fn deliver_held(&mut self) -> Result<(), HarnessError> {
        match self.held.take() {
            Some(input) => self.steer(&input).await,
            None => Ok(()),
        }
    }

    /// Wait out the steerability gate.
    ///
    /// Nothing else is reading the notification stream while `submit` runs, so
    /// this pumps it itself — `turn/started` arrives on that stream, and the
    /// events it absorbs on the way queue up for [`Self::next`] in native order.
    async fn await_steerable(&mut self) -> Result<(), HarnessError> {
        loop {
            match self.phase {
                TurnPhase::Running => return Ok(()),
                TurnPhase::Ended => return Err(not_steerable("the Turn had already ended")),
                TurnPhase::Requested => {
                    if !self.pump().await {
                        return Err(not_steerable("the Turn ended before it became steerable"));
                    }
                }
            }
        }
    }

    /// Send one message into the running Turn and report it accepted.
    ///
    /// `expectedTurnId` is a precondition the server checks: it must be the Turn
    /// currently held, never a remembered one. Every rejection uses the same
    /// `-32600` with a different sentence, so nothing here reads the message —
    /// any failure means the same thing, that this Turn cannot take the input.
    async fn steer(&mut self, input: &Input) -> Result<(), HarnessError> {
        let (Some(session), Some(thread_id), Some(turn_id)) = (
            self.session.as_ref(),
            self.thread_id.as_ref(),
            self.turn_id.as_ref(),
        ) else {
            return Err(not_steerable("the Turn has no identity to steer"));
        };
        session
            .steer(thread_id, turn_id, &input.text)
            .await
            .map_err(|error| classify::classify_request_error("turn/steer", &error))?;
        info!(input = %input.id.0, %turn_id, "Steered a message into the running Codex Turn");
        self.pending.push_back(ProcessEvent::InputAccepted {
            input: input.id.clone(),
        });
        Ok(())
    }

    /// Take the next server event off the stream and absorb it. Returns `false`
    /// once the stream is finished — its terminal, if it produced one, is already
    /// on the pending queue.
    async fn pump(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        match session.notifications.recv().await {
            Some(Ok(event)) => {
                self.absorb(event);
                true
            }
            Some(Err(error)) => {
                self.phase = TurnPhase::Ended;
                self.pending.push_back(ProcessEvent::Terminated {
                    termination: ProcessTermination::Failed(classify::classify_stream_error(
                        &error,
                    )),
                });
                false
            }
            // EOF with no terminal event: the driver maps this to an unexpected
            // exit.
            None => {
                self.phase = TurnPhase::Ended;
                false
            }
        }
    }

    /// Normalize one native item into its canonical block(s) and append each as
    /// a `ProcessEvent::Block` in native order (a tool item expands to a `tool_use`
    /// plus its paired `tool_result`). A family that renders nothing appends
    /// nothing.
    fn push_item_blocks(&mut self, item: &ThreadItem) {
        for source in self.normalizer.item_sources(item) {
            self.pending.push_back(ProcessEvent::Block(source));
        }
    }

    /// The terminal, and the authority on how the Turn ended.
    ///
    /// An interrupted Turn is reported as a normal end, not a failure: the
    /// driver owns cancelled-ness (a process never reports it itself) and maps
    /// any termination to `Cancelled` when it was the one that asked.
    fn absorb_turn_completed(&mut self, completed: &TurnCompletedNotification) {
        let nothing = TokenUsageBreakdown::default();
        let latest = self.usage_latest.as_ref().unwrap_or(&nothing);
        let usage = TurnUsage::between(self.usage_baseline.as_ref(), latest);
        self.pending.push_back(ProcessEvent::Block(
            self.normalizer.turn_result_source(&usage),
        ));
        // Turn totals for the backend's per-turn cost/token accounting. Codex
        // reports no cost, so the payload carries tokens and model only.
        self.pending
            .push_back(ProcessEvent::RunTotals(self.normalizer.run_totals(&usage)));

        let termination = match completed.turn.status {
            TurnStatus::Failed => {
                let error = completed.turn.error.clone().unwrap_or_else(|| TurnError {
                    message: "the Codex Turn failed without reporting a cause".to_string(),
                    codex_error_info: None,
                    additional_details: None,
                });
                ProcessTermination::Failed(self.classify_failure(&error))
            }
            TurnStatus::Completed | TurnStatus::Interrupted => ProcessTermination::Completed,
            // A terminal that says the Turn is still running, or a status this
            // build does not know: the Turn is over either way, so end it as
            // completed rather than inventing a failure.
            status => {
                warn!(?status, "Unexpected status on turn/completed");
                ProcessTermination::Completed
            }
        };
        self.phase = TurnPhase::Ended;
        self.pending
            .push_back(ProcessEvent::Terminated { termination });
    }

    /// Classify a failed Turn, flagging the Codex credential first when the
    /// named cause is an unauthorized request. The coordinator polls the flag at
    /// the turn-failure boundary and owns everything after it.
    fn classify_failure(&self, error: &TurnError) -> HarnessError {
        if classify::is_expired_credential(error) {
            info!("Codex reported an unauthorized request; flagging the credential");
            self.cfg.credential_failures.trigger(CODEX_AUTH_JSON);
        }
        classify::classify_turn_error(error)
    }

    /// Whether a notification belongs to the thread this process opened.
    ///
    /// A native sub-agent runs on a thread of its own and streams over the same
    /// connection, so a `turn/completed` on this pipe is not necessarily *this*
    /// Turn's terminal. Everything the sub-agent's thread emits belongs to that
    /// thread: its items are not this Turn's transcript, and its token totals
    /// are not this Turn's spend.
    ///
    /// **Fails open.** A notification the server attributed to no thread is
    /// taken as this one's. Dropping an unattributed `turn/completed` would
    /// leave the Turn with no terminal at all, which costs more than the
    /// mis-attribution it would prevent.
    fn owns_thread(&self, event: &ServerEvent) -> bool {
        let (Some(own), Some(event_thread)) = (self.thread_id.as_deref(), event.thread_id()) else {
            return true;
        };
        own == event_thread
    }

    /// Turn one server event into zero or more `ProcessEvent`s, appended to the
    /// pending queue in native order.
    fn absorb(&mut self, event: ServerEvent) {
        if !self.owns_thread(&event) {
            debug!(
                thread_id = event.thread_id(),
                own_thread_id = self.thread_id.as_deref(),
                "Ignoring a Codex notification from another thread"
            );
            return;
        }
        match event {
            ServerEvent::ThreadStarted(started) => {
                // A fresh thread announces itself twice — in the `thread/start`
                // response and here — but an injected test session only ever
                // sees this one, so the id every `turn/*` request needs is taken
                // from whichever arrives. A sub-agent's own announcement never
                // reaches here: `owns_thread` rejects it against the id the
                // response already set.
                self.thread_id = Some(started.thread.id.clone());
                self.announce_session(started.thread.id);
            }
            // The gate opens here, not at `turn/start`'s response.
            ServerEvent::TurnStarted(started) => {
                self.turn_id = Some(started.turn.id);
                self.phase = TurnPhase::Running;
            }
            // Every family commits its block on `item/completed`; nothing
            // re-emits on start (the running plan tracker, which did, is a
            // notification on this transport). Logged rather than dropped so a
            // Turn that stops making progress names what it stopped on.
            ServerEvent::ItemStarted(ItemNotification { item, .. }) => {
                debug!(
                    item_id = item.id(),
                    item_type = item.type_name(),
                    "Codex item started"
                );
            }
            ServerEvent::ItemCompleted(ItemNotification { item, .. }) => {
                self.push_item_blocks(&item);
            }
            ServerEvent::PlanUpdated(plan) => {
                for source in self.normalizer.plan_sources(&plan.plan) {
                    self.pending.push_back(ProcessEvent::Block(source));
                }
            }
            ServerEvent::TokenUsage(usage) => {
                // What occupies the window is the last request's own input and
                // output; `total` is the whole Session's spend and would pass the
                // window it is measured against. Clamped, so a reading that
                // cannot be right reads as full rather than as impossible.
                let used =
                    usage.token_usage.last.input_tokens + usage.token_usage.last.output_tokens;
                let max_tokens = usage.token_usage.model_context_window;
                self.pending
                    .push_back(ProcessEvent::ContextWindow(ContextWindow {
                        used_tokens: max_tokens.map_or(used, |max| used.min(max)),
                        max_tokens,
                    }));
                // Before the Turn opens, the reading is the Session's stored
                // total replayed on resume; from `turn/started` on, every further
                // reading is this Turn's own growth — including the model
                // requests a steered message added to it.
                if self.phase == TurnPhase::Requested {
                    self.usage_baseline = Some(usage.token_usage.total.clone());
                }
                self.usage_latest = Some(usage.token_usage.total);
            }
            // Not a terminal: the same error rides on `turn/completed`, which is
            // the authority, and `willRetry` means Codex is still working.
            ServerEvent::Error(error) => {
                warn!(
                    message = %error.error.message,
                    will_retry = error.will_retry,
                    "Codex reported a Turn error"
                );
            }
            ServerEvent::TurnCompleted(completed) => self.absorb_turn_completed(&completed),
        }
    }
}

#[async_trait::async_trait]
impl AgentProcess for CodexProcess {
    async fn submit(&mut self, input: Input) -> Result<(), HarnessError> {
        // The driver delivers the input that opens the execution first, so the
        // first submit is the Turn and every later one is a message that arrived
        // while it runs.
        if self.input_id.is_none() {
            // Set before opening, because `announce_session` pairs the Session
            // with the acceptance of this input and runs inside that call.
            self.input_id = Some(input.id.clone());
            // A session that is already open means a driver-level test injected
            // the stream — record the input against it instead of spawning a
            // real CLI.
            if self.session.is_none() {
                let thread_id = self.open_turn(&input).await?;
                self.announce_session(thread_id);
            }
            return Ok(());
        }

        self.held = Some(input);
        // The gate is the only part of this that can wait on something that may
        // never come, so the deadline and the Stop are raced against *it*, not
        // against the steer that follows. Abandoning a `turn/steer` whose
        // request is already written would hand the message back to the driver
        // while the server was still free to accept it, and it would be acted on
        // twice — once inside this Turn and once as the next one. The steer needs
        // no deadline of its own: it is a request against a live Turn, and the
        // stream ending releases its waiter (`codex_app_server::PendingRequests`).
        let cancelled = self.cancel_token.clone();
        let opened = tokio::select! {
            biased;
            () = cancelled.cancelled() => Err(not_steerable("the Turn was cancelled")),
            opened = tokio::time::timeout(MID_TURN_DELIVERY_TIMEOUT, self.await_steerable()) => {
                opened.unwrap_or_else(|_elapsed| {
                    Err(not_steerable("the Turn did not become steerable in time"))
                })
            }
        };
        let delivered = match opened {
            Ok(()) => self.deliver_held().await,
            Err(error) => Err(error),
        };
        if delivered.is_err() {
            // The driver takes back only the input it just submitted, so a
            // message left behind here would be one nobody owns.
            self.held = None;
        }
        delivered
    }

    /// A message that arrives while the Turn runs is folded into it, exactly as
    /// `ClaudeProcess` does, rather than waiting for a Turn of its own.
    fn accepts_injection(&self) -> bool {
        true
    }

    async fn cancel(&mut self) {
        // Ask the Turn to stop before killing the server, so Codex writes its
        // rollout and the Session stays resumable. Only possible once both ids
        // are known; before that, tearing the process down is the whole of it.
        if let Some(session) = self.session.as_ref() {
            if let (Some(thread_id), Some(turn_id)) = (&self.thread_id, &self.turn_id) {
                if let Err(error) = session.interrupt(thread_id, turn_id).await {
                    // The Turn is ending regardless; the interrupt is a courtesy
                    // and its failure must not stop the teardown below.
                    warn!(%error, "turn/interrupt failed; tearing the session down anyway");
                }
            }
            // The stream then reaches EOF/terminal and the driver maps it to a
            // cancelled outcome. Idempotent — the token cancel is a no-op if set.
            session.cancel();
        }
        // Also cancel through the process's own handle, which covers the window
        // before `open_turn` has a session to hand back. The real session shares
        // this token, so the two calls are the same cancel; only an injected
        // test session has one of its own.
        self.cancel_token.cancel();
    }

    fn cancel_handle(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    async fn next(&mut self) -> Option<ProcessEvent> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }
            if !self.pump().await {
                // The stream is finished; hand over the terminal it left, if any.
                return self.pending.pop_front();
            }
        }
    }
}

/// This Turn cannot take a mid-Turn message, whatever the reason — the gate had
/// not opened, the Turn had ended, or the server refused the steer. The driver
/// queues the message for the next Turn (`driver.rs`), which is the one and only
/// answer to all of them.
fn not_steerable(reason: &str) -> HarnessError {
    HarnessError::new(
        FailureStage::Input,
        ErrorClass::InvalidInput,
        SessionImpact::Intact,
        format!("the running Codex Turn could not take the message: {reason}"),
    )
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
