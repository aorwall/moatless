//! Production Codex CLI adapter for the Agent harness.
//!
//! This is the Codex implementation of the private [`AgentProcess`] port: one
//! `codex exec --json` subprocess per Turn (Codex is one-process-per-Turn, so it
//! rides the shared one-shot `driver::run`). A later Turn resumes a known thread
//! by replacing the process. The adapter launches the CLI, normalizes each
//! native thread item into a canonical per-block [`ProcessEvent`], and classifies
//! failures into stable [`HarnessError`] facts.
//!
//! Split for cohesion so no single file owns process lifecycle, native mapping,
//! and error policy at once:
//! - this module owns the process lifecycle (launch, cancel, stream draining);
//! - [`normalize`] turns native `CodexExecEvent`s into canonical `ProcessEvent`s;
//! - [`classify`] maps startup/stream/turn failures into `HarnessError` facts.
//!
//! What it deliberately does *not* do — because those belong to the backend or
//! the coordinator (ADR-0002, ADR-0003):
//! - it never assembles the `tool_use` / `tool_result` Message envelope from an
//!   item (one canonical `SourceEvent` per native item, in native order);
//! - it never silently falls back to a fresh thread when a resume fails — it
//!   reports [`ErrorClass::UnavailableSession`] with [`SessionImpact::Lost`] and
//!   lets the coordinator drive recovery.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::agent::codex_exec::{self, CodexCliSession, CodexExecEvent, ThreadItemDetails};
use crate::agent::types::AgentRunRequest;
use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;

use super::error::HarnessError;
use super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::signal::{Input, InputId, SessionId};

use self::normalize::Normalizer;

mod classify;
mod normalize;

#[cfg(test)]
mod tests;

/// The Turn-independent inputs a Codex subprocess needs. Per-Turn configuration
/// (prompt, model, resume) rides on the [`Input`]; credential and system-prompt
/// resolution is the coordinator's job, so a resolved `env`, `system_prompt`, and
/// shared `credential_failures` detector are passed in rather than fetched here.
#[derive(Clone)]
pub struct CodexConfig {
    pub config: Arc<Config>,
    pub env: Option<HashMap<String, String>>,
    pub system_prompt: Option<String>,
    /// Shared stderr-marker detector. Codex surfaces a refresh-token race on
    /// stderr; the detector flips a flag the coordinator polls to drive a
    /// credential pull + retry, keeping credential recovery behind the harness.
    pub credential_failures: CredentialFailureDetector,
}

/// One Codex CLI subprocess for a single Turn.
pub(crate) struct CodexProcess {
    cfg: CodexConfig,
    session: Option<CodexCliSession>,
    /// Pre-emitted events (a `SessionReady` is always paired with an
    /// `InputAccepted`) waiting to be drained one at a time by [`Self::next`].
    pending: VecDeque<ProcessEvent>,
    input_id: Option<InputId>,
    session_ready: bool,
    input_accepted: bool,
    normalizer: Normalizer,
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
        }
    }

    /// Build a process backed by an already-open (injected) session for
    /// deterministic driver-level lifecycle tests. `submit` then records the
    /// input against this session instead of spawning a real CLI.
    #[cfg(test)]
    pub(crate) fn with_session(cfg: CodexConfig, session: CodexCliSession, model: &str) -> Self {
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
            .is_some_and(CodexCliSession::is_cancelled)
    }

    /// Turn one Codex event into zero or more `ProcessEvent`s, appended to the
    /// pending queue in native order. The one-shot lifecycle: `thread.started`
    /// proves the process is up (Session known + input accepted at spawn), and a
    /// terminal `turn.completed` / `turn.failed` / `error` ends the single
    /// execution by ending the process.
    fn absorb(&mut self, event: CodexExecEvent) {
        match event {
            CodexExecEvent::ThreadStarted(started) => {
                let session = SessionId(started.thread_id);
                self.normalizer.set_session(session.clone());
                if !self.session_ready {
                    self.session_ready = true;
                    self.pending
                        .push_back(ProcessEvent::SessionReady { session });
                }
                // The one-shot CLI accepted the prompt at spawn; the thread start
                // proves the process is up, so acceptance is known now.
                if !self.input_accepted {
                    self.input_accepted = true;
                    if let Some(input) = self.input_id.clone() {
                        self.pending
                            .push_back(ProcessEvent::InputAccepted { input });
                    }
                }
            }
            // Turn boundary markers carry no committed content block.
            CodexExecEvent::TurnStarted(_) => {}
            // Only the running plan tracker re-emits on start/update; every other
            // family commits its single block on `item.completed`.
            CodexExecEvent::ItemStarted(event) | CodexExecEvent::ItemUpdated(event) => {
                if matches!(event.item.details, ThreadItemDetails::TodoList(_)) {
                    self.pending.push_back(ProcessEvent::Block(
                        self.normalizer.item_source(&event.item),
                    ));
                }
            }
            CodexExecEvent::ItemCompleted(event) => {
                self.pending.push_back(ProcessEvent::Block(
                    self.normalizer.item_source(&event.item),
                ));
            }
            CodexExecEvent::TurnCompleted(event) => {
                // Retain the turn-level usage as a canonical source block before
                // the terminal (which has no usage carrier).
                self.pending.push_back(ProcessEvent::Block(
                    self.normalizer.turn_result_source(&event.usage),
                ));
                self.pending.push_back(ProcessEvent::Terminated {
                    termination: ProcessTermination::Completed,
                });
            }
            CodexExecEvent::TurnFailed(event) => {
                self.pending.push_back(ProcessEvent::Terminated {
                    termination: ProcessTermination::Failed(classify::classify_turn_error(
                        &event.error.message,
                    )),
                });
            }
            CodexExecEvent::Error(event) => {
                self.pending.push_back(ProcessEvent::Terminated {
                    termination: ProcessTermination::Failed(classify::classify_turn_error(
                        &event.message,
                    )),
                });
            }
        }
    }
}

#[async_trait::async_trait]
impl AgentProcess for CodexProcess {
    async fn submit(&mut self, input: Input) -> Result<(), HarnessError> {
        // The driver mints a fresh process and calls `submit` exactly once, so a
        // session that is already open means a driver-level test injected the
        // stream — record the input against it instead of spawning a real CLI.
        if self.session.is_none() {
            // `start_fresh` always wins; otherwise the resume request
            // (`resume <id>` / `resume --last`) rides straight to the CLI. A
            // failed resume surfaces as a classified turn error, never a silent
            // fresh start.
            let resume = if input.start_fresh {
                None
            } else {
                input.resume.clone()
            };

            let request = AgentRunRequest {
                content: Vec::new(),
                model: input.model.clone(),
                system_prompt: self.cfg.system_prompt.clone(),
                resume,
                agent_type: Some("codex".to_string()),
                agent_mode: input.mode.clone(),
            };
            let model = codex_exec::resolve_codex_model(&self.cfg.config, &request);

            match codex_exec::start(
                &input.text,
                &self.cfg.config,
                &request,
                self.cfg.env.clone(),
                self.cfg.credential_failures.clone(),
            )
            .await
            {
                Ok(session) => {
                    self.normalizer = Normalizer::new(Some(model));
                    self.session = Some(session);
                }
                Err(error) => return Err(classify::classify_startup_error(&error)),
            }
        }
        self.input_id = Some(input.id);
        Ok(())
    }

    async fn cancel(&mut self) {
        // Cancelling the session tears down the codex process tree; the stream
        // then reaches EOF/terminal and the driver maps it to a cancelled
        // outcome. Idempotent — the underlying token cancel is a no-op if set.
        if let Some(session) = self.session.as_ref() {
            session.cancel();
        }
    }

    async fn next(&mut self) -> Option<ProcessEvent> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }
            let session = self.session.as_mut()?;
            match session.messages.recv().await {
                Some(Ok(event)) => self.absorb(event),
                Some(Err(error)) => {
                    return Some(ProcessEvent::Terminated {
                        termination: ProcessTermination::Failed(classify::classify_stream_error(
                            &error,
                        )),
                    });
                }
                // EOF with no terminal event: the driver maps this to an
                // unexpected exit.
                None => return None,
            }
        }
    }
}
