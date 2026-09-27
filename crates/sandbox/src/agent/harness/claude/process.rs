use super::{ClaudeConfig, classify, fork, session};
use std::collections::VecDeque;
use std::sync::Arc;

use claude_agent_sdk::api::content::{ApiMessage, ContentBlock, Usage};
use claude_agent_sdk::api::messages::{MessagePriority, ResultMessage, UserMessage};
use claude_agent_sdk::{QueryHandle, SdkMessage, query};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::agent::options::build_options;
use crate::agent::types::{AgentRunRequest, ResumeSession};

use crate::agent::harness::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use crate::agent::harness::process::{AgentProcess, ProcessEvent, ProcessTermination};
use crate::agent::harness::signal::{ContextWindow, Input, InputId, RunTotals, SessionId};
use crate::agent::harness::trajectory::ClaudeSessionFormat;
use crate::trajectory_tail::TurnTrajectory;

use super::normalize::{Normalizer, assistant_blocks, user_blocks};

/// What the turn's last main-thread response says about the context window.
///
/// `max_tokens` is the largest window across all models used in the run (a
/// sub-agent's smaller window must not decide the meter); `used_tokens` comes
/// only from `last_response` (sub-agent usage skipped), clamped to
/// `max_tokens`, and is `None` before any main-thread response lands.
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

/// The run's authoritative token and cost totals from the terminal result.
///
/// Cost is always `Some` (Claude prices the run itself); `model` is the
/// `modelUsage` key with the most output, used only as a label;
/// `reasoning_output_tokens` is always `0` since Claude reports none.
pub(super) fn run_totals(result: &ResultMessage) -> RunTotals {
    let (usage, model_usage, cost, num_turns, duration_ms, duration_api_ms) = match result {
        ResultMessage::Success(s) => (
            &s.usage,
            &s.model_usage,
            s.total_cost_usd,
            s.num_turns,
            s.duration_ms,
            s.duration_api_ms,
        ),
        ResultMessage::Error(e) => (
            &e.usage,
            &e.model_usage,
            e.total_cost_usd,
            e.num_turns,
            e.duration_ms,
            e.duration_api_ms,
        ),
    };
    let clamp = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    let model = model_usage
        .iter()
        .max_by_key(|(_, u)| u.output_tokens)
        .map(|(name, _)| name.clone());
    RunTotals {
        input_tokens: clamp(usage.input_tokens),
        output_tokens: clamp(usage.output_tokens),
        cache_creation_input_tokens: clamp(usage.cache_creation_input_tokens.unwrap_or(0)),
        cache_read_input_tokens: clamp(usage.cache_read_input_tokens.unwrap_or(0)),
        reasoning_output_tokens: 0,
        total_cost_usd: Some(cost),
        num_turns: Some(num_turns),
        duration_ms: Some(duration_ms),
        duration_api_ms: Some(duration_api_ms),
        model,
        model_usage: serde_json::to_value(model_usage).ok(),
    }
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
    trajectory: Option<TurnTrajectory>,
}

/// Build the `UserMessage` frame for an input injected into a running Session.
///
/// `Next` is load-bearing: it folds the message into the running execution (as
/// an extra internal turn) while still emitting exactly one `Result`, which is
/// what lets the one-shot transport carry an injected input at all; `Now` means
/// *interrupt* — it abandons in-flight work and returns an empty result, so it
/// must not be used here.
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
            trajectory: None,
        }
    }

    /// Turn one `SdkMessage` into zero or more `ProcessEvent`s, appended to the
    /// pending queue in native order.
    ///
    /// One-shot lifecycle: the first `System` init proves the process is up
    /// (Session known + input accepted at spawn), and the terminal `Result`
    /// ends the single execution by ending the process.
    fn absorb(&mut self, msg: SdkMessage) {
        match msg {
            SdkMessage::System(init) => {
                let session = SessionId(init.session_id.clone());
                self.session_id = Some(init.session_id.clone());
                self.normalizer.set_session(session.clone());
                if let Some(trajectory) = self.trajectory.as_mut() {
                    trajectory.record(&init.session_id);
                }
                if !self.session_ready {
                    self.session_ready = true;
                    // Claude carries no native turn id; a fork cuts its transcript
                    // by Message uuid instead.
                    self.pending.push_back(ProcessEvent::SessionReady {
                        session,
                        agent_turn_id: None,
                    });
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
                for event in assistant_blocks(&assistant) {
                    self.pending.push_back(ProcessEvent::Block(event));
                }
            }
            SdkMessage::User(user) => {
                for event in user_blocks(&user) {
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
                // The terminal `ResultMessage` is the only trustworthy source of
                // the run's cost and token totals: Claude's per-message
                // `output_tokens` is sampled before the response finished.
                self.pending
                    .push_back(ProcessEvent::RunTotals(run_totals(&result)));
                self.pending.push_back(ProcessEvent::Terminated {
                    termination: classify::classify_result(&result),
                });
            }
            // Partial streaming frames, replays, and system subtypes carry no
            // committed content block for this path.
            _ => {}
        }
    }

    /// Resolve the resume mode for this input.
    ///
    /// `start_fresh` always wins; a `Last` (`--continue`) request downgrades to
    /// fresh when there is no on-disk transcript, because `--continue` against
    /// an empty project dir makes the CLI exit immediately.
    async fn resolve_resume(&self, input: &Input) -> Option<ResumeSession> {
        // A fork's first Turn resumes a truncated copy of the parent's
        // transcript, not the parent's Session, and rides ahead of `start_fresh`
        // — which a fork also sets, as the version-skew guard for a pod too old
        // to know the `fork` field. When the cut succeeds the copy is the resume
        // target; when it cannot (no transcript on disk, or the anchor is
        // missing) the fork falls through to a fresh Session.
        if let Some(fork) = &input.fork {
            if let Some(anchor) = &fork.source_message_uuid {
                if let Some(new_session_id) = fork::write_forked_transcript(
                    &self.cfg.config.workspace_dir,
                    &fork.source_session_id,
                    anchor,
                )
                .await
                {
                    return Some(ResumeSession::WithId(new_session_id));
                }
            }
            return None;
        }
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

        self.trajectory = TurnTrajectory::new(
            self.cfg.trajectory.as_ref(),
            Arc::new(ClaudeSessionFormat),
            &self.cfg.config,
            &self.cfg.config.claude_config_dir,
            &input,
        );
        let fork_session = match self.trajectory.as_mut() {
            Some(trajectory) => trajectory.write_out(&input).await?,
            None => None,
        };
        session::ensure_explicit_resume_is_available(
            &self.cfg.config,
            &input.resume,
            input.start_fresh,
        )
        .await?;
        let resume = match fork_session {
            Some(id) => Some(ResumeSession::WithId(id)),
            None => self.resolve_resume(&input).await,
        };
        self.resuming = resume.is_some();

        let request = AgentRunRequest {
            content: Vec::new(),
            model: input.model.clone(),
            system_prompt: input.system_prompt.clone(),
            resume,
            agent_profile: self.cfg.agent_profile.clone(),
            agent_mode: input.mode.clone(),
            effort: input.effort.clone(),
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
        let event = self.next_event().await;
        if matches!(event, None | Some(ProcessEvent::Terminated { .. })) {
            if let Some(trajectory) = &self.trajectory {
                trajectory.flush().await;
            }
        }
        event
    }
}

impl ClaudeProcess {
    async fn next_event(&mut self) -> Option<ProcessEvent> {
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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
