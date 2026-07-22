//! Production Claude subprocess adapter for the Agent harness.
//!
//! This is the Claude implementation of the private [`AgentProcess`] port: one
//! CLI subprocess per Turn, driven through the `claude-agent-sdk` one-shot
//! `query` transport (the persistent tmux transport is ticket 06). It launches
//! the CLI, normalizes each native `SdkMessage` into canonical per-block
//! [`ProcessEvent`]s, and classifies failures into stable [`HarnessError`]
//! facts.
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

use std::collections::HashMap;
use std::sync::Arc;

use claude_agent_sdk::types::content::ContentBlock;
use claude_agent_sdk::types::messages::{
    AssistantMessage, ResultMessage, ToolProgressMessage, UserMessage,
};
use claude_agent_sdk::{query, QueryHandle, SdkError, SdkMessage};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::agent::agent_state::AgentState;
use crate::agent::options::build_options;
use crate::agent::types::{AgentRunRequest, ResumeSession};
use crate::config::Config;

use super::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::signal::{Activity, Input, InputId, SessionId};
use super::source::{NativeMessageId, Role, SourceEvent, SourceId};

/// Upper bound on a diagnostic string carried in a [`HarnessError`].
const MAX_DIAGNOSTIC: usize = 500;

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

/// The per-message metadata shared by every content block of one native
/// `SdkMessage`, so a whole assistant/user envelope's blocks are normalized with
/// one grouping id, parent, usage, and model.
struct BlockMeta {
    session: SessionId,
    message_id: Option<String>,
    parent_tool_use_id: Option<String>,
    role: Role,
    usage: Option<serde_json::Value>,
    model: Option<String>,
}

/// One Claude CLI subprocess for a single Turn.
pub(crate) struct ClaudeProcess {
    cfg: ClaudeConfig,
    cancel_token: CancellationToken,
    handle: Option<QueryHandle>,
    /// Pre-emitted events (a `SessionReady` is always paired with an
    /// `InputAccepted`) waiting to be drained one at a time by [`Self::next`].
    pending: std::collections::VecDeque<ProcessEvent>,
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
            pending: std::collections::VecDeque::new(),
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

    /// Build the canonical source event for one native content block, assigning a
    /// collision-safe id and preserving native grouping/usage/model.
    fn make_source(&mut self, meta: &BlockMeta, block: &ContentBlock) -> SourceEvent {
        let seq = self.block_seq;
        self.block_seq += 1;
        let native = meta
            .message_id
            .clone()
            .unwrap_or_else(|| format!("{}-anon-{seq}", self.nonce));
        SourceEvent {
            source_id: SourceId(format!("{native}#{seq}")),
            message_id: NativeMessageId(native),
            session: meta.session.clone(),
            parent_tool_use_id: meta.parent_tool_use_id.clone(),
            role: meta.role,
            block: serde_json::to_value(block).unwrap_or(serde_json::Value::Null),
            usage: meta.usage.clone(),
            model: meta.model.clone(),
            diagnostics: None,
        }
    }

    /// The Session as currently known, falling back to a native id when the
    /// process reports a block before its `System` init (defensive; the driver
    /// buffers early blocks regardless).
    fn session_or(&self, native: &str) -> SessionId {
        self.session
            .clone()
            .unwrap_or_else(|| SessionId(native.to_string()))
    }

    /// Turn one `SdkMessage` into zero or more `ProcessEvent`s, appended to the
    /// pending queue in native order.
    fn absorb(&mut self, msg: SdkMessage) {
        match msg {
            SdkMessage::System(init) => {
                let session = SessionId(init.session_id.clone());
                self.session = Some(session.clone());
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
            SdkMessage::Assistant(assistant) => self.absorb_assistant(assistant),
            SdkMessage::User(user) => self.absorb_user(user),
            SdkMessage::ToolProgress(progress) => self.absorb_progress(progress),
            SdkMessage::Result(result) => {
                self.pending.push_back(ProcessEvent::Terminated {
                    termination: classify_result(&result),
                });
            }
            // Partial streaming frames, replays, and system subtypes carry no
            // committed content block for this path.
            _ => {}
        }
    }

    fn absorb_assistant(&mut self, assistant: AssistantMessage) {
        let meta = BlockMeta {
            session: self.session_or(&assistant.session_id),
            message_id: assistant.message.id.clone(),
            parent_tool_use_id: assistant.parent_tool_use_id.clone(),
            role: Role::Assistant,
            usage: assistant
                .message
                .usage
                .as_ref()
                .and_then(|u| serde_json::to_value(u).ok()),
            model: assistant.message.model.clone(),
        };
        for block in &assistant.message.content {
            let event = self.make_source(&meta, block);
            self.pending.push_back(ProcessEvent::Block(event));
        }
    }

    fn absorb_user(&mut self, user: UserMessage) {
        let meta = BlockMeta {
            session: self.session_or(&user.session_id),
            message_id: user.uuid.clone(),
            parent_tool_use_id: user.parent_tool_use_id.clone(),
            role: Role::User,
            usage: None,
            model: None,
        };
        for block in &user.message.content {
            let event = self.make_source(&meta, block);
            self.pending.push_back(ProcessEvent::Block(event));
        }
    }

    fn absorb_progress(&mut self, progress: ToolProgressMessage) {
        self.pending.push_back(ProcessEvent::Activity(Activity {
            session: self.session_or(&progress.session_id),
            detail: progress.tool_name,
        }));
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
            Err(error) => Err(classify_startup_error(&error, self.resuming)),
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
                        termination: ProcessTermination::Failed(classify_stream_error(&error)),
                    });
                }
                // EOF with no `Result`: the driver maps this to an unexpected exit.
                None => return None,
            }
        }
    }
}

fn truncate_diagnostic(text: &str) -> String {
    if text.len() <= MAX_DIAGNOSTIC {
        text.to_string()
    } else {
        format!("{}…", &text[..MAX_DIAGNOSTIC])
    }
}

/// Whether a startup failure means the resume-target transcript is unavailable
/// (a stale session id after a pod restart), the case the coordinator recovers
/// from by starting fresh. Mirrors the legacy `is_session_unavailable_error`.
fn is_session_unavailable(error: &SdkError) -> bool {
    matches!(
        error,
        SdkError::TmuxSessionDied { .. }
            | SdkError::SessionStartTimeout { .. }
            | SdkError::ProcessExit { .. }
            | SdkError::JsonlNotFound { .. }
    )
}

fn classify_sdk_error(error: &SdkError) -> (ErrorClass, SessionImpact) {
    match error {
        SdkError::CliNotFound { .. } | SdkError::InvalidConfig { .. } | SdkError::SpawnError(_) => {
            (ErrorClass::InvalidConfiguration, SessionImpact::Unknown)
        }
        SdkError::JsonError(_) | SdkError::JsonParseError { .. } => {
            (ErrorClass::NativeProtocol, SessionImpact::Unknown)
        }
        SdkError::ControlTimeout { .. } | SdkError::SessionStartTimeout { .. } => {
            (ErrorClass::UnresponsiveProcess, SessionImpact::Unknown)
        }
        SdkError::ProcessExit { .. } | SdkError::TransportClosed | SdkError::Cancelled => {
            (ErrorClass::ProcessExit, SessionImpact::Unknown)
        }
        SdkError::JsonlNotFound { .. } | SdkError::TmuxSessionDied { .. } => {
            (ErrorClass::UnavailableSession, SessionImpact::Lost)
        }
        SdkError::TmuxNotFound
        | SdkError::HookFifoFailed(_)
        | SdkError::HookHarnessFailed(_)
        | SdkError::ControlError { .. }
        | SdkError::Internal(_) => (ErrorClass::Internal, SessionImpact::Unknown),
    }
}

fn classify_startup_error(error: &SdkError, resuming: bool) -> HarnessError {
    if resuming && is_session_unavailable(error) {
        return HarnessError::new(
            FailureStage::Startup,
            ErrorClass::UnavailableSession,
            SessionImpact::Lost,
            truncate_diagnostic(&error.to_string()),
        );
    }
    let (class, impact) = classify_sdk_error(error);
    HarnessError::new(
        FailureStage::Startup,
        class,
        impact,
        truncate_diagnostic(&error.to_string()),
    )
}

fn classify_stream_error(error: &SdkError) -> HarnessError {
    let (class, impact) = classify_sdk_error(error);
    HarnessError::new(
        FailureStage::Execution,
        class,
        impact,
        truncate_diagnostic(&error.to_string()),
    )
}

/// Classify a terminal `ResultMessage`: a clean success completes the execution,
/// any error result becomes a classified failure the coordinator maps to policy.
fn classify_result(result: &ResultMessage) -> ProcessTermination {
    match result {
        ResultMessage::Success(success) if !success.is_error => ProcessTermination::Completed,
        ResultMessage::Success(success) => {
            ProcessTermination::Failed(result_failure("", &success.result))
        }
        ResultMessage::Error(error) => {
            ProcessTermination::Failed(result_failure(&error.subtype, &error.errors.join("; ")))
        }
    }
}

fn result_failure(subtype: &str, text: &str) -> HarnessError {
    let haystack = format!("{subtype} {text}").to_lowercase();
    let matches_any = |patterns: &[&str]| patterns.iter().any(|p| haystack.contains(p));
    let class = if matches_any(&[
        "overloaded",
        "rate_limit",
        "capacity",
        "server_error",
        "529",
        "503",
        "502",
    ]) {
        ErrorClass::RateLimit
    } else if matches_any(&[
        "authenticat",
        "unauthorized",
        "401",
        "invalid api key",
        "invalid_api_key",
        "credit balance",
    ]) {
        ErrorClass::Authentication
    } else {
        // A completed-but-error result is the CLI rejecting the input (e.g.
        // "Unknown command: ..."), not a process/config failure.
        ErrorClass::InvalidInput
    };
    HarnessError::new(
        FailureStage::Execution,
        class,
        SessionImpact::Intact,
        truncate_diagnostic(text),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_agent_sdk::types::content::{ApiMessage, Usage};
    use claude_agent_sdk::types::messages::{ResultError, ResultSuccess, SystemInitMessage};

    fn process() -> ClaudeProcess {
        ClaudeProcess::new(ClaudeConfig {
            config: Arc::new(crate::config::test_support::minimal_config()),
            agent_state: Arc::new(AgentState::new()),
            env: None,
            system_prompt: None,
            agent_type: Some("claude-code".to_string()),
        })
    }

    fn drain(p: &mut ClaudeProcess) -> Vec<ProcessEvent> {
        let mut out = Vec::new();
        while let Some(event) = p.pending.pop_front() {
            out.push(event);
        }
        out
    }

    fn usage() -> Usage {
        Usage {
            input_tokens: 2,
            output_tokens: 3,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            service_tier: None,
            cache_creation: None,
            server_tool_use: None,
        }
    }

    fn assistant_line(id: Option<&str>, block: ContentBlock, with_usage: bool) -> AssistantMessage {
        AssistantMessage {
            uuid: "row".to_string(),
            session_id: "sess".to_string(),
            message: ApiMessage {
                role: "assistant".to_string(),
                content: vec![block],
                usage: with_usage.then(usage),
                model: Some("claude-opus-4-8".to_string()),
                id: id.map(ToString::to_string),
                stop_reason: None,
                stop_sequence: None,
            },
            parent_tool_use_id: None,
            error: None,
            is_api_error_message: None,
            api_error: None,
            request_id: None,
        }
    }

    fn system_init(session: &str) -> SystemInitMessage {
        SystemInitMessage {
            uuid: "u".to_string(),
            session_id: session.to_string(),
            tools: vec![],
            model: "claude-opus-4-8".to_string(),
            mcp_servers: vec![],
            permission_mode: claude_agent_sdk::PermissionMode::Default,
            cwd: "/workspace".to_string(),
            api_key_source: claude_agent_sdk::types::messages::ApiKeySource::None,
            slash_commands: vec![],
            output_style: claude_agent_sdk::types::messages::OutputStyle::Normal,
            skills: vec![],
            plugins: vec![],
            agents: None,
            betas: None,
            claude_code_version: "1".to_string(),
        }
    }

    fn text(s: &str) -> ContentBlock {
        ContentBlock::Text {
            text: s.to_string(),
        }
    }

    #[test]
    fn system_init_yields_session_ready_then_input_accepted() {
        let mut p = process();
        p.input_id = Some(InputId("in-1".to_string()));
        p.absorb(SdkMessage::System(system_init("sess-real")));
        let events = drain(&mut p);
        assert!(matches!(
            events[0],
            ProcessEvent::SessionReady { ref session } if session.0 == "sess-real"
        ));
        assert!(matches!(
            events[1],
            ProcessEvent::InputAccepted { ref input } if input.0 == "in-1"
        ));
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn each_content_block_is_one_source_event_in_native_order() {
        let mut p = process();
        p.absorb(SdkMessage::System(system_init("sess")));
        drain(&mut p);
        // One assistant response, two blocks sharing a message id.
        p.absorb(SdkMessage::Assistant(assistant_line(
            Some("msg_1"),
            text("first"),
            true,
        )));
        p.absorb(SdkMessage::Assistant(assistant_line(
            Some("msg_1"),
            text("second"),
            true,
        )));
        let events = drain(&mut p);
        let sources: Vec<&SourceEvent> = events
            .iter()
            .filter_map(|e| match e {
                ProcessEvent::Block(s) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(sources.len(), 2);
        // Same native grouping id, distinct collision-safe source ids, order kept.
        assert_eq!(sources[0].message_id, sources[1].message_id);
        assert_ne!(sources[0].source_id, sources[1].source_id);
        assert_eq!(sources[0].block["text"], "first");
        assert_eq!(sources[1].block["text"], "second");
    }

    #[test]
    fn repeated_usage_is_forwarded_on_every_block_not_coalesced() {
        let mut p = process();
        p.absorb(SdkMessage::Assistant(assistant_line(
            Some("msg_1"),
            text("a"),
            true,
        )));
        p.absorb(SdkMessage::Assistant(assistant_line(
            Some("msg_1"),
            text("b"),
            true,
        )));
        let events = drain(&mut p);
        for event in &events {
            if let ProcessEvent::Block(s) = event {
                assert!(s.usage.is_some(), "usage forwarded verbatim per block");
                assert_eq!(s.model.as_deref(), Some("claude-opus-4-8"));
            }
        }
    }

    #[test]
    fn tool_progress_is_activity_not_content() {
        let mut p = process();
        p.absorb(SdkMessage::System(system_init("sess")));
        drain(&mut p);
        p.absorb(SdkMessage::ToolProgress(ToolProgressMessage {
            uuid: "u".to_string(),
            session_id: "sess".to_string(),
            tool_use_id: "t".to_string(),
            tool_name: "Bash".to_string(),
            parent_tool_use_id: None,
            elapsed_time_seconds: 12.0,
        }));
        let events = drain(&mut p);
        assert!(matches!(
            events.as_slice(),
            [ProcessEvent::Activity(a)] if a.detail == "Bash" && a.session.0 == "sess"
        ));
    }

    #[test]
    fn clean_result_completes() {
        let mut p = process();
        p.absorb(SdkMessage::Result(ResultMessage::Success(ResultSuccess {
            uuid: "u".to_string(),
            session_id: "sess".to_string(),
            duration_ms: 1,
            duration_api_ms: 1,
            is_error: false,
            num_turns: 1,
            result: "done".to_string(),
            total_cost_usd: 0.0,
            usage: usage(),
            model_usage: std::collections::HashMap::new(),
            permission_denials: vec![],
            structured_output: None,
        })));
        let events = drain(&mut p);
        assert!(matches!(
            events.as_slice(),
            [ProcessEvent::Terminated {
                termination: ProcessTermination::Completed
            }]
        ));
    }

    #[test]
    fn rate_limited_error_result_classifies_as_rate_limit() {
        let result = ResultMessage::Error(ResultError {
            subtype: "rate_limit".to_string(),
            uuid: "u".to_string(),
            session_id: "sess".to_string(),
            duration_ms: 1,
            duration_api_ms: 1,
            is_error: true,
            num_turns: 1,
            total_cost_usd: 0.0,
            usage: usage(),
            model_usage: std::collections::HashMap::new(),
            permission_denials: vec![],
            errors: vec!["HTTP 429 overloaded".to_string()],
        });
        match classify_result(&result) {
            ProcessTermination::Failed(error) => {
                assert_eq!(error.class, ErrorClass::RateLimit);
                assert_eq!(error.session_impact, SessionImpact::Intact);
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn success_with_is_error_is_invalid_input() {
        let result = ResultMessage::Success(ResultSuccess {
            uuid: "u".to_string(),
            session_id: "sess".to_string(),
            duration_ms: 1,
            duration_api_ms: 1,
            is_error: true,
            num_turns: 1,
            result: "Unknown command: /foo".to_string(),
            total_cost_usd: 0.0,
            usage: usage(),
            model_usage: std::collections::HashMap::new(),
            permission_denials: vec![],
            structured_output: None,
        });
        match classify_result(&result) {
            ProcessTermination::Failed(error) => assert_eq!(error.class, ErrorClass::InvalidInput),
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn resume_startup_failure_is_unavailable_session_and_lost() {
        let error = SdkError::ProcessExit {
            code: Some(1),
            signal: None,
        };
        let classified = classify_startup_error(&error, true);
        assert_eq!(classified.class, ErrorClass::UnavailableSession);
        assert_eq!(classified.session_impact, SessionImpact::Lost);
        assert_eq!(classified.stage, FailureStage::Startup);
    }

    #[test]
    fn non_resume_process_exit_stays_a_process_exit() {
        let error = SdkError::ProcessExit {
            code: Some(1),
            signal: None,
        };
        let classified = classify_startup_error(&error, false);
        assert_eq!(classified.class, ErrorClass::ProcessExit);
    }

    #[test]
    fn cli_not_found_is_invalid_configuration() {
        let error = SdkError::CliNotFound {
            path: "/bin/claude".to_string(),
        };
        assert_eq!(
            classify_startup_error(&error, false).class,
            ErrorClass::InvalidConfiguration
        );
    }

    #[test]
    fn malformed_protocol_is_native_protocol() {
        let error = SdkError::JsonParseError {
            source: serde_json::from_str::<serde_json::Value>("{").unwrap_err(),
            raw_line: "{".to_string(),
        };
        assert_eq!(
            classify_stream_error(&error).class,
            ErrorClass::NativeProtocol
        );
    }

    /// Focused real-CLI E2E: launch the actual `claude` subprocess through the
    /// production [`super::super::Harness::claude`] seam and assert the external
    /// signal ordering for a trivial Turn. Gated behind `CLAUDE_SDK_E2E=1` and
    /// `#[ignore]` — the full E2E matrix runs at ticket 10, per the guide.
    #[tokio::test]
    #[ignore = "requires the real claude CLI and API credentials (CLAUDE_SDK_E2E=1)"]
    async fn e2e_claude_subprocess_turn_through_the_harness() {
        if std::env::var("CLAUDE_SDK_E2E").as_deref() != Ok("1") {
            eprintln!("skipping: set CLAUDE_SDK_E2E=1 to run");
            return;
        }
        use super::super::signal::{Outcome, Signal};
        use super::super::Harness;

        let mut config = crate::config::test_support::minimal_config();
        let workspace = std::env::temp_dir().join("harness-claude-e2e");
        std::fs::create_dir_all(&workspace).expect("create workspace dir");
        config.workspace_dir = workspace.to_string_lossy().into_owned();
        config.repo_dir = config.workspace_dir.clone();
        if let Ok(path) = std::env::var("SANDBOX_CLAUDE_EXECUTABLE_PATH") {
            config.claude_cli_path = path;
        }
        config.default_model = Some("claude-haiku-4-5-20251001".to_string());

        let mut env = HashMap::new();
        if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
            env.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token);
        }
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            env.insert("ANTHROPIC_API_KEY".to_string(), key);
        }

        let cfg = ClaudeConfig {
            config: Arc::new(config),
            agent_state: Arc::new(AgentState::new()),
            env: (!env.is_empty()).then_some(env),
            system_prompt: None,
            agent_type: Some("claude-code".to_string()),
        };

        let mut harness = Harness::claude(cfg);
        harness
            .submit(Input::text(
                "in-1",
                "What is 2+2? Reply with only the number.",
            ))
            .await
            .expect("submit input");

        let (mut started, mut accepted, mut sources) = (false, false, 0usize);
        let mut outcome = None;
        while let Some(signal) = harness.next_signal().await {
            match signal {
                Signal::ExecutionStarted { .. } => started = true,
                Signal::InputAccepted { .. } => accepted = true,
                Signal::Source(_) => sources += 1,
                Signal::ExecutionFinished { outcome: finished } => {
                    outcome = Some(finished);
                    break;
                }
                _ => {}
            }
        }

        assert!(started, "expected ExecutionStarted");
        assert!(accepted, "expected InputAccepted");
        assert!(sources > 0, "expected at least one source block");
        assert!(
            matches!(outcome, Some(Outcome::Completed)),
            "expected Completed, got {outcome:?}"
        );
    }
}
