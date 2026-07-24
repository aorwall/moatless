//! Production Claude subprocess adapter for the Agent harness.
//!
//! This is the Claude implementation of the private [`AgentProcess`] port: one
//! CLI subprocess per Turn, driven through the `claude-agent-sdk` one-shot
//! `query` transport (the persistent tmux transport is ticket 06). It launches
//! the CLI, normalizes each native `SdkMessage` into canonical per-block
//! [`ProcessEvent`]s, and classifies failures into stable [`HarnessError`]
//! facts.
//!
//! The adapter is split for cohesion, so no single file owns process lifecycle,
//! native mapping, and error policy at once:
//! - this module owns the process lifecycle (launch, cancel, stream draining);
//! - [`normalize`] turns native `SdkMessage`s into canonical `ProcessEvent`s;
//! - [`classify`] maps SDK/result failures into stable `HarnessError` facts.
//!
//! What it deliberately does *not* do — because those belong to the backend or
//! the coordinator, not the harness (ADR-0002, ADR-0003, ADR-0005):
//! - it never folds content blocks into an assembled Message (one `SourceEvent`
//!   per native block, in native order);
//! - it forwards repeated usage verbatim on every block (the backend admits it
//!   once) rather than deduplicating like the legacy `AssistantBuffer`;
//! - it never silently falls back to a fresh Session on resume failure — it
//!   reports [`ErrorClass::UnavailableSession`] with [`SessionImpact::Lost`] and
//!   lets the coordinator drive the context-loss recovery.

use std::collections::VecDeque;
use std::sync::Arc;

use claude_agent_sdk::{query, QueryHandle, SdkMessage};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::agent::agent_state::AgentState;
use crate::agent::options::build_options;
use crate::agent::types::{AgentRunRequest, ResumeSession};
use crate::config::Config;

use super::error::HarnessError;
use super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::signal::{Input, InputId, SessionId};

use self::normalize::Normalizer;

mod classify;
mod normalize;
mod persistent;

pub(crate) use persistent::ClaudePersistentProcess;

#[cfg(test)]
mod tests;

/// The Turn-independent inputs a Claude subprocess needs. All per-Turn
/// configuration — prompt, model, mode, resume, and the coordinator-resolved
/// `env` / `system_prompt` — rides on the [`Input`], so this config stays fixed
/// for the harness lifetime.
#[derive(Clone)]
pub struct ClaudeConfig {
    pub config: Arc<Config>,
    pub agent_state: Arc<AgentState>,
    pub agent_type: Option<String>,
}

/// One Claude CLI subprocess for a single Turn.
pub(crate) struct ClaudeProcess {
    cfg: ClaudeConfig,
    cancel_token: CancellationToken,
    handle: Option<QueryHandle>,
    /// Pre-emitted events (a `SessionReady` is always paired with an
    /// `InputAccepted`) waiting to be drained one at a time by [`Self::next`].
    pending: VecDeque<ProcessEvent>,
    input_id: Option<InputId>,
    session_ready: bool,
    input_accepted: bool,
    /// Whether this launch requested a resume — decides whether a startup
    /// failure is a lost Session or a plain process/config failure.
    resuming: bool,
    /// Shared native-block normalizer (collision-safe ids + Session tracking).
    normalizer: Normalizer,
}

impl ClaudeProcess {
    pub(crate) fn new(cfg: ClaudeConfig) -> Self {
        Self {
            cfg,
            cancel_token: CancellationToken::new(),
            handle: None,
            pending: VecDeque::new(),
            input_id: None,
            session_ready: false,
            input_accepted: false,
            resuming: false,
            normalizer: Normalizer::new(),
        }
    }

    /// Turn one `SdkMessage` into zero or more `ProcessEvent`s, appended to the
    /// pending queue in native order. The one-shot lifecycle: the first `System`
    /// init proves the process is up (Session known + input accepted at spawn),
    /// and the terminal `Result` ends the single execution by ending the process.
    fn absorb(&mut self, msg: SdkMessage) {
        match msg {
            SdkMessage::System(init) => {
                let session = SessionId(init.session_id.clone());
                self.normalizer.set_session(session.clone());
                if !self.session_ready {
                    self.session_ready = true;
                    self.pending
                        .push_back(ProcessEvent::SessionReady { session });
                }
                // The one-shot CLI accepted the prompt at spawn; the init proves
                // the process is up, so acceptance is known now.
                if !self.input_accepted {
                    self.input_accepted = true;
                    if let Some(input) = self.input_id.clone() {
                        self.pending
                            .push_back(ProcessEvent::InputAccepted { input });
                    }
                }
            }
            SdkMessage::Assistant(assistant) => {
                for event in self.normalizer.assistant_blocks(&assistant) {
                    self.pending.push_back(ProcessEvent::Block(event));
                }
            }
            SdkMessage::User(user) => {
                for event in self.normalizer.user_blocks(&user) {
                    self.pending.push_back(ProcessEvent::Block(event));
                }
            }
            SdkMessage::ToolProgress(progress) => {
                self.pending.push_back(ProcessEvent::Activity(
                    self.normalizer.progress_activity(&progress),
                ));
            }
            SdkMessage::Result(result) => {
                self.pending.push_back(ProcessEvent::Terminated {
                    termination: classify::classify_result(&result),
                });
            }
            // Partial streaming frames, replays, and system subtypes carry no
            // committed content block for this path.
            _ => {}
        }
    }

    /// Resolve the resume mode for this input. `start_fresh` always wins; a
    /// `Last` (`--continue`) request is downgraded to fresh when there is no
    /// on-disk transcript, matching the legacy launch path (a `--continue`
    /// against an empty project dir makes the CLI exit immediately).
    async fn resolve_resume(&self, input: &Input) -> Option<ResumeSession> {
        if input.start_fresh {
            return None;
        }
        match &input.resume {
            Some(ResumeSession::WithId(id)) => Some(ResumeSession::WithId(id.clone())),
            Some(ResumeSession::Last) => {
                match claude_agent_sdk::has_resumable_transcript(Some(
                    &self.cfg.config.workspace_dir,
                ))
                .await
                {
                    Ok(false) => None,
                    Ok(true) => Some(ResumeSession::Last),
                    Err(error) => {
                        warn!(error = %error, "transcript probe failed; keeping --continue");
                        Some(ResumeSession::Last)
                    }
                }
            }
            None => None,
        }
    }
}

#[async_trait::async_trait]
impl AgentProcess for ClaudeProcess {
    async fn submit(&mut self, input: Input) -> Result<(), HarnessError> {
        let resume = self.resolve_resume(&input).await;
        self.resuming = resume.is_some();

        let request = AgentRunRequest {
            content: Vec::new(),
            model: input.model.clone(),
            system_prompt: input.system_prompt.clone(),
            resume,
            agent_type: self.cfg.agent_type.clone(),
            agent_mode: input.mode.clone(),
        };
        let options = build_options(
            &self.cfg.config,
            &request,
            self.cancel_token.clone(),
            Arc::clone(&self.cfg.agent_state),
            input.env.clone(),
        );

        match query(&input.text, options).await {
            Ok(handle) => {
                self.input_id = Some(input.id);
                self.handle = Some(handle);
                Ok(())
            }
            Err(error) => Err(classify::classify_startup_error(&error, self.resuming)),
        }
    }

    async fn cancel(&mut self) {
        if let Some(handle) = self.handle.as_ref() {
            // Graceful in-band turn stop first; the token cancel then guarantees
            // the process tree is torn down so the stream reaches its terminal.
            if let Err(error) = handle.interrupt().await {
                warn!(error = %error, "claude interrupt failed; forcing teardown");
            }
            handle.close();
        }
        self.cancel_token.cancel();
    }

    async fn next(&mut self) -> Option<ProcessEvent> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }
            let handle = self.handle.as_mut()?;
            match handle.messages.recv().await {
                Some(Ok(msg)) => self.absorb(msg),
                Some(Err(error)) => {
                    return Some(ProcessEvent::Terminated {
                        termination: ProcessTermination::Failed(classify::classify_stream_error(
                            &error,
                        )),
                    });
                }
                // EOF with no `Result`: the driver maps this to an unexpected exit.
                None => return None,
            }
        }
    }
}
