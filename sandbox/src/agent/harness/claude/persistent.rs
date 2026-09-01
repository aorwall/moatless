//! Persistent Claude (tmux) adapter for the Agent harness.
//!
//! One long-lived TUI session hosts many Turns behind the private
//! [`AgentProcess`] port. A compatible Input is pasted into the live session
//! (injected into an active Turn, or opening a fresh one while idle); an
//! incompatible model / mode / Session / fresh-start replaces the process,
//! resuming the intended Session on a new TUI. Turns the CLI starts on its own
//! (an internal `ScheduleWakeup` firing or a `Monitor` event) surface as
//! harness-internal executions.
//!
//! All native-payload mapping is delegated to the shared [`Normalizer`]; this
//! file owns only the persistent process lifecycle and Turn-boundary state, so
//! neither the coordinator nor the driver learns anything tmux-specific.

use std::collections::VecDeque;
use std::sync::Arc;

use claude_agent_sdk::api::content::{ApiMessage, ContentBlock};
use claude_agent_sdk::api::messages::{MessagePriority, UserMessage};
use claude_agent_sdk::{QueryHandle, SdkMessage, query_tmux_streaming};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::agent::options::build_options;
use crate::agent::types::{AgentRunRequest, ResumeSession};

use super::super::error::HarnessError;
use super::super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::super::signal::{Input, InputId, SessionId};
use super::ClaudeConfig;
use super::classify::{classify_result, classify_startup_error, classify_stream_error};
use super::normalize::Normalizer;

/// The per-Input configuration the live session was opened with. A new Input
/// compatible with this pastes into the live TUI; an incompatible one forces a
/// replacement (kill + respawn with `--resume`). Agent type and system prompt
/// are fixed for the Sandbox lifetime (the fixed-harness decision), so only model, mode, effort,
/// and the resumed Session id can diverge between Inputs.
struct Fingerprint {
    model: Option<String>,
    mode: Option<String>,
    /// Effort is a launch flag, so a change forces a replacement rather than a
    /// paste into the live session.
    effort: Option<String>,
    /// The live Session id (empty until the first `System` init reports it).
    session_id: String,
}

/// A persistent Claude TUI session behind the harness seam.
pub(crate) struct ClaudePersistentProcess {
    cfg: ClaudeConfig,
    handle: Option<QueryHandle>,
    /// Session-level cancel token for the live TUI; recreated on every spawn.
    cancel_token: CancellationToken,
    normalizer: Normalizer,
    fingerprint: Option<Fingerprint>,
    /// A Turn is streaming — between its start (paste / internal) and its
    /// terminal `Result`.
    turn_in_flight: bool,
    /// Set when a fresh spawn / replacement is waiting for the first `System`
    /// init to announce `SessionReady` + acceptance for the opening Input.
    awaiting_init: Option<InputId>,
    /// Synthesized events drained before reading the transport.
    pending: VecDeque<ProcessEvent>,
    /// Wakes [`Self::next`] once [`Self::submit`] has created the transport.
    handle_ready: Notify,
}

impl ClaudePersistentProcess {
    pub(crate) fn new(cfg: ClaudeConfig) -> Self {
        Self {
            cfg,
            handle: None,
            cancel_token: CancellationToken::new(),
            normalizer: Normalizer::new(),
            fingerprint: None,
            turn_in_flight: false,
            awaiting_init: None,
            pending: VecDeque::new(),
            handle_ready: Notify::new(),
        }
    }

    /// Resolve the resume target for a (re)spawn. `replacing` carries the live
    /// Session id when this spawn replaces an existing session so the new TUI
    /// continues the same conversation.
    async fn spawn_resume(&self, input: &Input, replacing: Option<&str>) -> Option<ResumeSession> {
        // A fork's first Turn resumes a truncated copy of the parent's
        // transcript under a fresh id, not the parent's Session and not the live
        // one — so it rides ahead of both `start_fresh` (which a fork also sets,
        // as the version-skew guard) and `replacing`. When the cut succeeds the
        // copy is the resume target; when it cannot (no transcript on disk, or
        // the anchor is missing) the fork falls through to a fresh Session. This
        // is the same cut `ClaudeProcess::resolve_resume` makes for the
        // subprocess transport.
        if let Some(fork) = &input.fork {
            if let Some(anchor) = &fork.source_message_uuid {
                if let Some(new_session_id) = super::fork::write_forked_transcript(
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
        if let Some(ResumeSession::WithId(id)) = &input.resume {
            if !id.is_empty() {
                return Some(ResumeSession::WithId(id.clone()));
            }
        }
        if let Some(sid) = replacing.filter(|s| !s.is_empty()) {
            return Some(ResumeSession::WithId(sid.to_string()));
        }
        match &input.resume {
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
            _ => None,
        }
    }

    /// Spawn a fresh TUI and paste the opening prompt. On any failure the partial
    /// session is torn down and a classified startup error is returned.
    async fn spawn(&mut self, input: &Input, replacing: Option<&str>) -> Result<(), HarnessError> {
        let resume = self.spawn_resume(input, replacing).await;
        let resuming = resume.is_some();
        let request = AgentRunRequest {
            content: Vec::new(),
            model: input.model.clone(),
            system_prompt: input.system_prompt.clone(),
            resume,
            agent_type: self.cfg.agent_type.clone(),
            agent_mode: input.mode.clone(),
            effort: input.effort.clone(),
        };
        self.cancel_token = CancellationToken::new();
        let options = build_options(
            &self.cfg.config,
            &request,
            self.cancel_token.clone(),
            Arc::clone(&self.cfg.agent_state),
            input.env.clone(),
            self.cfg.events.clone(),
        );

        let handle = query_tmux_streaming(options)
            .await
            .map_err(|error| classify_startup_error(&error, resuming))?;
        let paste_session = replacing.unwrap_or_default();
        if let Err(error) = handle
            .send_user_message(&make_user_message(&input.text, paste_session))
            .await
        {
            handle.close();
            return Err(classify_startup_error(&error, resuming));
        }

        self.fingerprint = Some(Fingerprint {
            model: input.model.clone(),
            mode: input.mode.clone(),
            effort: input.effort.clone(),
            session_id: paste_session.to_string(),
        });
        self.handle = Some(handle);
        self.handle_ready.notify_one();
        Ok(())
    }

    /// Paste `input` into the live TUI without respawning.
    async fn paste(&self, input: &Input) -> Result<(), HarnessError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| classify_stream_error(&claude_agent_sdk::SdkError::TransportClosed))?;
        let session = self
            .fingerprint
            .as_ref()
            .map(|f| f.session_id.clone())
            .unwrap_or_default();
        handle
            .send_user_message(&make_user_message(&input.text, &session))
            .await
            .map_err(|error| classify_stream_error(&error))
    }

    /// Fully terminate the live TUI before starting a replacement, preserving the
    /// at-most-one-process invariant.
    async fn teardown(&mut self) {
        if let Some(handle) = self.handle.take() {
            if let Err(error) = handle.interrupt().await {
                warn!(error = %error, "claude interrupt failed during replacement; forcing teardown");
            }
            handle.close();
        }
        self.cancel_token.cancel();
        self.turn_in_flight = false;
    }

    /// Turn one `SdkMessage` into zero or more `ProcessEvent`s, appended to the
    /// pending queue in native order.
    fn absorb(&mut self, msg: SdkMessage) {
        match msg {
            SdkMessage::System(init) => {
                let session = SessionId(init.session_id.clone());
                self.normalizer.set_session(session.clone());
                if let Some(fp) = self.fingerprint.as_mut() {
                    fp.session_id = init.session_id.clone();
                }
                // A fresh spawn / replacement announces its start only now that
                // the real Session id is known.
                if let Some(input) = self.awaiting_init.take() {
                    self.pending.push_back(ProcessEvent::SessionReady {
                        session,
                        agent_turn_id: None,
                    });
                    self.pending
                        .push_back(ProcessEvent::InputAccepted { input });
                    self.turn_in_flight = true;
                }
            }
            SdkMessage::Assistant(assistant) => {
                self.begin_internal_turn_if_idle(&assistant.session_id);
                for event in self.normalizer.assistant_blocks(&assistant) {
                    self.pending.push_back(ProcessEvent::Block(event));
                }
            }
            SdkMessage::User(user) => {
                self.begin_internal_turn_if_idle(&user.session_id);
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
                // Turn boundary: the session persists to host the next Turn.
                self.pending.push_back(ProcessEvent::ExecutionEnded {
                    termination: classify_result(&result),
                });
                self.turn_in_flight = false;
            }
            _ => {}
        }
    }

    /// Content arriving while no Turn is in flight is a Turn the CLI started on
    /// its own: announce a harness-internal execution before its first block.
    fn begin_internal_turn_if_idle(&mut self, native_session: &str) {
        if !self.turn_in_flight {
            let session = self
                .normalizer
                .session()
                .unwrap_or_else(|| SessionId(native_session.to_string()));
            self.pending
                .push_back(ProcessEvent::InternalStart { session });
            self.turn_in_flight = true;
        }
    }
}

#[async_trait::async_trait]
impl AgentProcess for ClaudePersistentProcess {
    async fn submit(&mut self, input: Input) -> Result<(), HarnessError> {
        let input_id = input.id.clone();

        // No live session yet: spawn one and paste the opening prompt.
        if self.handle.is_none() {
            super::session::ensure_explicit_resume_is_available(
                &self.cfg.config,
                &input.resume,
                input.start_fresh,
            )
            .await?;
            self.spawn(&input, None).await?;
            self.awaiting_init = Some(input_id);
            // Suppress internal-turn detection on this Turn's blocks; the real
            // start is announced when the System init arrives.
            self.turn_in_flight = true;
            return Ok(());
        }

        // Incompatible config: replace the process, resuming the live Session.
        if needs_restart(self.fingerprint.as_ref(), &input) {
            let resume_target = self.fingerprint.as_ref().map(|f| f.session_id.clone());
            if self.turn_in_flight {
                // End the abandoned Turn before the replacement starts.
                self.pending.push_back(ProcessEvent::ExecutionEnded {
                    termination: ProcessTermination::Completed,
                });
            }
            self.teardown().await;
            self.spawn(&input, resume_target.as_deref()).await?;
            self.awaiting_init = Some(input_id);
            self.turn_in_flight = true;
            return Ok(());
        }

        // Compatible with the live session.
        self.paste(&input).await?;
        if self.turn_in_flight {
            // Injected into the active Turn: acceptance only, no new start.
            self.pending
                .push_back(ProcessEvent::InputAccepted { input: input_id });
        } else {
            // Opens a new Turn on the same Session while idle.
            let session = self
                .fingerprint
                .as_ref()
                .map(|f| SessionId(f.session_id.clone()))
                .or_else(|| self.normalizer.session())
                .unwrap_or_else(|| SessionId(String::new()));
            self.pending.push_back(ProcessEvent::SessionReady {
                session,
                agent_turn_id: None,
            });
            self.pending
                .push_back(ProcessEvent::InputAccepted { input: input_id });
            self.turn_in_flight = true;
        }
        Ok(())
    }

    async fn cancel(&mut self) {
        // Interrupt the active Turn; the CLI emits a terminal `Result` the driver
        // maps to a cancelled outcome, and the TUI persists for the next Turn.
        if let Some(handle) = self.handle.as_ref() {
            if let Err(error) = handle.interrupt().await {
                warn!(error = %error, "claude interrupt failed; forcing teardown");
                handle.close();
            }
        }
    }

    async fn next(&mut self) -> Option<ProcessEvent> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }
            // No transport yet: wait until a submit spawns one.
            if self.handle.is_none() {
                self.handle_ready.notified().await;
                continue;
            }
            let received = if let Some(handle) = self.handle.as_mut() {
                handle.messages.recv().await
            } else {
                continue;
            };
            match received {
                Some(Ok(msg)) => self.absorb(msg),
                Some(Err(error)) => {
                    self.handle = None;
                    return Some(ProcessEvent::Terminated {
                        termination: ProcessTermination::Failed(classify_stream_error(&error)),
                    });
                }
                // Transport EOF: the persistent process itself has ended.
                None => {
                    self.handle = None;
                    return None;
                }
            }
        }
    }
}

/// Whether `input` cannot run inside the live session and needs a replacement
/// (kill + respawn with `--resume`). Uses the same fingerprint fields as tmux.
fn needs_restart(fingerprint: Option<&Fingerprint>, input: &Input) -> bool {
    if input.start_fresh {
        return true;
    }
    let Some(fp) = fingerprint else {
        return false;
    };
    if input.model != fp.model || input.mode != fp.mode || input.effort != fp.effort {
        return true;
    }
    if let Some(ResumeSession::WithId(id)) = &input.resume {
        if !id.is_empty() && id != &fp.session_id {
            return true;
        }
    }
    false
}

/// Build the `UserMessage` pasted into the TUI. The CLI expects the message's
/// `session_id` to match the live session so `--resume` can replay it.
fn make_user_message(text: &str, session_id: &str) -> UserMessage {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn live(effort: Option<&str>) -> Fingerprint {
        Fingerprint {
            model: Some("claude-sonnet-4-5".to_string()),
            mode: None,
            effort: effort.map(str::to_string),
            session_id: "sess-1".to_string(),
        }
    }

    fn input(effort: Option<&str>) -> Input {
        Input {
            model: Some("claude-sonnet-4-5".to_string()),
            effort: effort.map(str::to_string),
            ..Input::text("in-1", "hi")
        }
    }

    /// `--effort` is a launch flag, so the live TUI cannot change it: a Turn that
    /// asks for a different one has to be run on a replacement process.
    #[test]
    fn a_changed_effort_forces_a_replacement() {
        assert!(needs_restart(
            Some(&live(Some("high"))),
            &input(Some("max"))
        ));
        assert!(needs_restart(Some(&live(None)), &input(Some("high"))));
        assert!(needs_restart(Some(&live(Some("high"))), &input(None)));
    }

    #[test]
    fn the_same_effort_pastes_into_the_live_session() {
        assert!(!needs_restart(
            Some(&live(Some("high"))),
            &input(Some("high"))
        ));
        assert!(!needs_restart(Some(&live(None)), &input(None)));
    }
}
