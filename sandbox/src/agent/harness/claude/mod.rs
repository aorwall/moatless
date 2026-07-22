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

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use claude_agent_sdk::{query, QueryHandle};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::agent::agent_state::AgentState;
use crate::agent::options::build_options;
use crate::agent::types::{AgentRunRequest, ResumeSession};
use crate::config::Config;

use super::error::HarnessError;
use super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::signal::{Input, InputId, SessionId};

mod classify;
mod normalize;

#[cfg(test)]
mod tests;

/// The Turn-independent inputs a Claude subprocess needs. Per-Turn configuration
/// (prompt, model, mode, resume) rides on the [`Input`]; credential and system
/// prompt resolution is the coordinator's job, so a resolved `env` and
/// `system_prompt` are passed in rather than fetched here.
#[derive(Clone)]
pub struct ClaudeConfig {
    pub config: Arc<Config>,
    pub agent_state: Arc<AgentState>,
    pub env: Option<HashMap<String, String>>,
    pub system_prompt: Option<String>,
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
    session: Option<SessionId>,
    session_ready: bool,
    input_accepted: bool,
    /// Whether this launch requested a resume — decides whether a startup
    /// failure is a lost Session or a plain process/config failure.
    resuming: bool,
    /// Monotonic block counter: keeps every emitted `source_id` unique within
    /// this process even when the CLI reuses a native `message.id`.
    block_seq: u64,
    /// Per-process collision-avoidance element for blocks with no native id.
    nonce: String,
}

impl ClaudeProcess {
    pub(crate) fn new(cfg: ClaudeConfig) -> Self {
        Self {
            cfg,
            cancel_token: CancellationToken::new(),
            handle: None,
            pending: VecDeque::new(),
            input_id: None,
            session: None,
            session_ready: false,
            input_accepted: false,
            resuming: false,
            block_seq: 0,
            nonce: uuid::Uuid::new_v4().simple().to_string(),
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
            system_prompt: self.cfg.system_prompt.clone(),
            resume,
            agent_type: self.cfg.agent_type.clone(),
            agent_mode: input.mode.clone(),
        };
        let options = build_options(
            &self.cfg.config,
            &request,
            self.cancel_token.clone(),
            Arc::clone(&self.cfg.agent_state),
            self.cfg.env.clone(),
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
