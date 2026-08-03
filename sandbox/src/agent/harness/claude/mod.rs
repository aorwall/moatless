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
//! the coordinator, not the harness (server-owned message assembly, the agent.message skeleton, the harness-outcome boundary):
//! - it never folds content blocks into an assembled Message (one `SourceEvent`
//!   per native block, in native order);
//! - it forwards repeated usage verbatim on every block (the backend admits it
//!   once) rather than deduplicating in the sandbox;
//! - it never silently falls back to a fresh Session on resume failure — it
//!   reports [`ErrorClass::UnavailableSession`] with [`SessionImpact::Lost`] and
//!   lets the coordinator drive the context-loss recovery.

use std::collections::VecDeque;
use std::sync::Arc;

use claude_agent_sdk::types::content::{ApiMessage, ContentBlock, Usage};
use claude_agent_sdk::types::messages::{MessagePriority, ResultMessage, UserMessage};
use claude_agent_sdk::{query, QueryHandle, SdkMessage};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::agent::agent_state::AgentState;
use crate::agent::options::build_options;
use crate::agent::types::{AgentRunRequest, ResumeSession};
use crate::config::Config;

use super::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::signal::{ContextWindow, Input, InputId, SessionId};

use self::normalize::Normalizer;

/// What the turn's last model response says about the context window.
///
/// The maximum only ever exists on the provider's wire — no column holds one —
/// and the terminal `ResultMessage` carries it per model, as a running maximum.
/// The largest is the window the turn ran in; a sub-agent model with a smaller
/// one would otherwise decide the meter.
///
/// Used is that last response's own usage. For one API request the input,
/// cache-creation and cache-read counts are its prompt and `output_tokens` is
/// its completion, so the four together are what occupies the window once it
/// returns. The token counts beside the maximum are running totals instead —
/// cost accumulates in the same object — so they measure what the turn
/// processed, and a cached prefix read on every request of a long turn takes
/// them past the window many times over.
///
/// Sub-agent responses are skipped: they occupy their own window, not this one.
/// Clamped to the maximum, so a reading that cannot be right reads as full
/// rather than as impossible. `None` before any response has landed — a turn
/// with nothing to say rather than an empty window.
fn context_window(result: &ResultMessage, last_response: Option<&Usage>) -> Option<ContextWindow> {
    let model_usage = match result {
        ResultMessage::Success(success) => &success.model_usage,
        ResultMessage::Error(error) => &error.model_usage,
    };
    let max_tokens = model_usage
        .values()
        .map(|usage| usage.context_window)
        .max()
        .filter(|window| *window > 0)
        .map(|window| i64::try_from(window).unwrap_or(i64::MAX));

    let usage = last_response?;
    let used = usage.input_tokens
        + usage.cache_creation_input_tokens.unwrap_or(0)
        + usage.cache_read_input_tokens.unwrap_or(0)
        + usage.output_tokens;
    let used = i64::try_from(used).unwrap_or(i64::MAX);
    Some(ContextWindow {
        used_tokens: max_tokens.map_or(used, |max| used.min(max)),
        max_tokens,
    })
}

mod classify;
mod normalize;
mod persistent;
mod session;

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
    /// Publishes the interactive calls the permission handler parks on. `None`
    /// in tests, which drive the harness without a backend to publish to.
    pub events: Option<crate::events::GrpcEventPublisher>,
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
    /// Session id reported by the CLI's `System` init, needed to address a
    /// mid-execution injection at the running Session.
    session_id: Option<String>,
    /// Whether this launch requested a resume — decides whether a startup
    /// failure is a lost Session or a plain process/config failure.
    resuming: bool,
    /// Shared native-block normalizer (collision-safe ids + Session tracking).
    normalizer: Normalizer,
    /// Usage from the newest main-thread model response, which is what the
    /// context window holds. See [`context_window`].
    last_response: Option<Usage>,
}

/// Build the `UserMessage` frame for an input injected into a running Session.
///
/// `Next` is load-bearing: the CLI folds the message into the execution already
/// running (reporting it as an extra internal turn) and still emits exactly one
/// `Result`, which is what lets the one-shot transport — whose orchestrator ends
/// the stream at the first `Result` — carry an injected input at all. `Now`
/// means *interrupt*: it abandons the in-flight work and returns an empty result
/// immediately, so it must not be used here.
fn user_message(text: &str, session_id: &str) -> UserMessage {
    UserMessage {
        uuid: None,
        session_id: session_id.to_string(),
        message: ApiMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            usage: None,
            model: None,
            id: None,
            stop_reason: None,
            stop_sequence: None,
        },
        parent_tool_use_id: None,
        is_synthetic: None,
        tool_use_result: None,
        is_meta: None,
        is_compact_summary: None,
        is_visible_in_transcript_only: None,
        plan_content: None,
        thinking_metadata: None,
        todos: None,
        summarize_metadata: None,
        source_tool_assistant_uuid: None,
        source_tool_use_id: None,
        permission_mode: None,
        priority: MessagePriority::Next,
    }
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
            session_id: None,
            resuming: false,
            normalizer: Normalizer::new(),
            last_response: None,
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
                self.session_id = Some(init.session_id.clone());
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
                if assistant.parent_tool_use_id.is_none() {
                    if let Some(usage) = assistant.message.usage.clone() {
                        self.last_response = Some(usage);
                    }
                }
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
                if let Some(window) = context_window(&result, self.last_response.as_ref()) {
                    self.pending.push_back(ProcessEvent::ContextWindow(window));
                }
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
    /// on-disk transcript, because `--continue` against an empty project dir
    /// makes the CLI exit immediately.
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
        // A live handle means the execution opened by an earlier input is still
        // running: write this input into that Session instead of launching a
        // second CLI. It joins that execution and shares its single `Result`
        // (see `user_message`). Acceptance is known as soon as the write lands,
        // so the bare `InputAccepted` reports the injection with no
        // `SessionReady` ahead of it.
        if let Some(handle) = self.handle.as_ref() {
            let session_id = self.session_id.clone().unwrap_or_default();
            handle
                .send_user_message(&user_message(&input.text, &session_id))
                .await
                .map_err(|error| {
                    HarnessError::new(
                        FailureStage::Input,
                        ErrorClass::ProcessExit,
                        SessionImpact::Intact,
                        format!("failed to inject input into the running Session: {error}"),
                    )
                })?;
            self.pending
                .push_back(ProcessEvent::InputAccepted { input: input.id });
            return Ok(());
        }

        session::ensure_explicit_resume_is_available(
            &self.cfg.config,
            &input.resume,
            input.start_fresh,
        )
        .await?;
        let resume = self.resolve_resume(&input).await;
        self.resuming = resume.is_some();

        let request = AgentRunRequest {
            content: Vec::new(),
            model: input.model.clone(),
            system_prompt: input.system_prompt.clone(),
            resume,
            agent_type: self.cfg.agent_type.clone(),
            agent_mode: input.mode.clone(),
            effort: input.effort.clone(),
        };
        let options = build_options(
            &self.cfg.config,
            &request,
            self.cancel_token.clone(),
            Arc::clone(&self.cfg.agent_state),
            input.env.clone(),
            self.cfg.events.clone(),
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

    fn accepts_injection(&self) -> bool {
        true
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
