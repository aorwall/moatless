//! Unit + gated E2E coverage for the Claude subprocess adapter: normalization
//! (per-block source events, activity, termination) and failure classification.

use std::collections::HashMap;
use std::sync::Arc;

use claude_agent_sdk::api::content::{ApiMessage, ContentBlock, ModelUsage, Usage};
use claude_agent_sdk::api::messages::{
    AssistantMessage, ResultError, ResultMessage, ResultSuccess, SystemInitMessage,
    ToolProgressMessage,
};
use claude_agent_sdk::{SdkError, SdkMessage};

use crate::agent::agent_state::AgentState;

use super::super::error::{ErrorClass, FailureStage, SessionImpact};
use super::super::process::{ProcessEvent, ProcessTermination};
use super::super::signal::{Input, InputId};
use super::super::source::SourceEvent;
use super::classify::{classify_result, classify_startup_error, classify_stream_error};
use super::{ClaudeConfig, ClaudeProcess};

fn process() -> ClaudeProcess {
    ClaudeProcess::new(ClaudeConfig {
        config: Arc::new(crate::config::test_support::minimal_config()),
        agent_state: Arc::new(AgentState::new()),
        agent_type: Some("claude-code".to_string()),
        events: None,
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
        api_key_source: claude_agent_sdk::api::messages::ApiKeySource::None,
        slash_commands: vec![],
        output_style: claude_agent_sdk::api::messages::OutputStyle::Normal,
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
        ProcessEvent::SessionReady { ref session, .. } if session.0 == "sess-real"
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
    assert_eq!(sources[0].block_json()["text"], "first");
    assert_eq!(sources[1].block_json()["text"], "second");
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

/// The CLI reports an unreachable provider through the same is_error result as a
/// rejected command, so the text is the only thing separating "your pod has no
/// egress" from "your input was bad". Classifying these as `InvalidInput` both
/// mislabels the outage and makes it terminal, since only some classes retry.
#[test]
fn unreachable_provider_is_connectivity_not_invalid_input() {
    for text in [
        "API Error: Unable to connect to API (ENOTIMP)",
        "getaddrinfo ENOTFOUND api.anthropic.com",
        "could not resolve host: api.anthropic.com",
        "connect ECONNREFUSED 10.0.0.1:443",
        "socket hang up",
    ] {
        let result = ResultMessage::Success(ResultSuccess {
            uuid: "u".to_string(),
            session_id: "sess".to_string(),
            duration_ms: 1,
            duration_api_ms: 1,
            is_error: true,
            num_turns: 1,
            result: text.to_string(),
            total_cost_usd: 0.0,
            usage: usage(),
            model_usage: std::collections::HashMap::new(),
            permission_denials: vec![],
            structured_output: None,
        });
        match classify_result(&result) {
            ProcessTermination::Failed(error) => {
                assert_eq!(
                    error.class,
                    ErrorClass::Connectivity,
                    "{text:?} should classify as connectivity"
                );
            }
            other => panic!("expected failure for {text:?}, got {other:?}"),
        }
    }
}

/// One model response, with the counts that make up what it left in the window.
fn responded(prompt: u64, cached: u64, written: u64, subagent: bool) -> AssistantMessage {
    let mut line = assistant_line(Some("msg_1"), text("a"), true);
    line.message.usage = Some(Usage {
        input_tokens: prompt,
        output_tokens: written,
        cache_read_input_tokens: Some(cached),
        cache_creation_input_tokens: None,
        service_tier: None,
        cache_creation: None,
        server_tool_use: None,
    });
    if subagent {
        line.parent_tool_use_id = Some("toolu_agent".to_string());
    }
    line
}

/// A turn's per-model report: a running total of everything it processed, and
/// the window it ran in.
fn processed(total: u64, window: u64) -> HashMap<String, ModelUsage> {
    HashMap::from([(
        "claude-opus-4-8".to_string(),
        ModelUsage {
            input_tokens: total,
            output_tokens: total,
            cache_read_input_tokens: total,
            cache_creation_input_tokens: total,
            web_search_requests: 0,
            cost_usd: 12.0,
            context_window: window,
            max_output_tokens: 32_000,
        },
    )])
}

fn finished(model_usage: HashMap<String, ModelUsage>) -> SdkMessage {
    SdkMessage::Result(ResultMessage::Success(ResultSuccess {
        uuid: "u".to_string(),
        session_id: "sess".to_string(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        result: "done".to_string(),
        total_cost_usd: 0.0,
        usage: usage(),
        model_usage,
        permission_denials: vec![],
        structured_output: None,
    }))
}

fn reading(events: &[ProcessEvent]) -> Option<(i64, Option<i64>)> {
    events.iter().find_map(|event| match event {
        ProcessEvent::ContextWindow(window) => Some((window.used_tokens, window.max_tokens)),
        _ => None,
    })
}

/// What occupies the window is the newest response's own prompt and completion.
/// The turn's running totals count a cached prefix again on every request, so a
/// long turn passes the window it is measured against many times over.
#[test]
fn the_meter_reads_what_the_last_response_left_in_the_window() {
    let mut p = process();
    p.absorb(SdkMessage::Assistant(responded(1_000, 40_000, 500, false)));
    p.absorb(SdkMessage::Assistant(responded(1_200, 50_000, 800, false)));
    drain(&mut p);
    p.absorb(finished(processed(1_000_000, 200_000)));
    assert_eq!(
        reading(&drain(&mut p)),
        Some((52_000, Some(200_000))),
        "the newest response, not the turn's running total"
    );
}

/// A sub-agent fills its own window, not this one.
#[test]
fn a_sub_agents_response_does_not_move_the_meter() {
    let mut p = process();
    p.absorb(SdkMessage::Assistant(responded(1_000, 40_000, 500, false)));
    p.absorb(SdkMessage::Assistant(responded(9_000, 90_000, 900, true)));
    drain(&mut p);
    p.absorb(finished(processed(1_000, 200_000)));
    assert_eq!(reading(&drain(&mut p)), Some((41_500, Some(200_000))));
}

/// A reading that cannot be right reads as full rather than as impossible.
#[test]
fn a_reading_past_the_window_reads_as_full() {
    let mut p = process();
    p.absorb(SdkMessage::Assistant(responded(300_000, 0, 100, false)));
    drain(&mut p);
    p.absorb(finished(processed(1_000, 200_000)));
    assert_eq!(reading(&drain(&mut p)), Some((200_000, Some(200_000))));
}

/// A window nobody reported is a count with no percentage, not a percentage of
/// a guess.
#[test]
fn a_turn_whose_window_went_unreported_still_reports_what_it_holds() {
    let mut p = process();
    p.absorb(SdkMessage::Assistant(responded(1_000, 0, 500, false)));
    drain(&mut p);
    p.absorb(finished(HashMap::new()));
    assert_eq!(reading(&drain(&mut p)), Some((1_500, None)));
}

/// Nothing answered, so there is nothing to say about the window — which is not
/// the same as saying it is empty.
#[test]
fn a_turn_that_never_answered_reports_no_reading() {
    let mut p = process();
    p.absorb(finished(processed(1_000, 200_000)));
    assert_eq!(reading(&drain(&mut p)), None);
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
fn resume_transport_close_is_unavailable_session_and_lost() {
    let classified = classify_startup_error(&SdkError::TransportClosed, true);
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
    use super::super::Harness;
    use super::super::signal::{Outcome, Signal};

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
        agent_type: Some("claude-code".to_string()),
        events: None,
    };

    let mut harness = Harness::claude(cfg);
    let mut input = Input::text("in-1", "What is 2+2? Reply with only the number.");
    input.env = (!env.is_empty()).then_some(env);
    harness.submit(input).await.expect("submit input");

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

/// Focused real-CLI E2E for the persistent tmux transport: launch the actual
/// TUI through the production [`super::super::Harness::claude_persistent`] seam,
/// run one Turn, then submit a second Turn to prove the same live Session hosts
/// it (no replacement). Gated behind `CLAUDE_TMUX_E2E=1` (and needs `tmux` on
/// PATH); the full E2E matrix runs at ticket 10.
#[tokio::test]
#[ignore = "requires the real claude CLI + tmux + credentials (CLAUDE_TMUX_E2E=1)"]
async fn e2e_persistent_claude_two_turns_share_a_session() {
    if std::env::var("CLAUDE_TMUX_E2E").as_deref() != Ok("1") {
        eprintln!("skipping: set CLAUDE_TMUX_E2E=1 to run");
        return;
    }
    use super::super::Harness;
    use super::super::signal::{Cause, Outcome, SessionId, Signal};

    let mut config = crate::config::test_support::minimal_config();
    let workspace = std::env::temp_dir().join("harness-claude-persistent-e2e");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    config.workspace_dir = workspace.to_string_lossy().into_owned();
    config.repo_dir = config.workspace_dir.clone();
    config.use_tmux = true;
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

    let env = (!env.is_empty()).then_some(env);

    let cfg = ClaudeConfig {
        config: Arc::new(config),
        agent_state: Arc::new(AgentState::new()),
        agent_type: Some("claude-code".to_string()),
        events: None,
    };

    let mut harness = Harness::claude_persistent(cfg);

    // Runs one Turn and returns (session, outcome) for it.
    async fn run_turn(
        harness: &mut Harness,
        env: &Option<HashMap<String, String>>,
        id: &str,
        text: &str,
    ) -> (Option<SessionId>, Option<Outcome>) {
        let mut input = Input::text(id, text);
        input.env = env.clone();
        harness.submit(input).await.expect("submit input");
        let (mut session, mut outcome) = (None, None);
        while let Some(signal) = harness.next_signal().await {
            match signal {
                Signal::ExecutionStarted {
                    session: s, cause, ..
                } => {
                    assert!(
                        matches!(cause, Cause::Input(_)),
                        "user Turn is input-caused"
                    );
                    session = Some(s);
                }
                Signal::ExecutionFinished { outcome: finished } => {
                    outcome = Some(finished);
                    break;
                }
                _ => {}
            }
        }
        (session, outcome)
    }

    let (first_session, first_outcome) = run_turn(
        &mut harness,
        &env,
        "in-1",
        "What is 2+2? Reply with only the number.",
    )
    .await;
    assert!(
        matches!(first_outcome, Some(Outcome::Completed)),
        "first Turn should complete, got {first_outcome:?}"
    );
    let first_session = first_session.expect("first Turn reports a Session");

    let (second_session, second_outcome) = run_turn(
        &mut harness,
        &env,
        "in-2",
        "And what is that number times 3?",
    )
    .await;
    assert!(
        matches!(second_outcome, Some(Outcome::Completed)),
        "second Turn should complete, got {second_outcome:?}"
    );
    assert_eq!(
        second_session.expect("second Turn reports a Session"),
        first_session,
        "the second Turn must run on the same persistent Session"
    );
}
