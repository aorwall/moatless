//! Real-CLI E2E, gated on `CODEX_SDK_E2E=1`.

use super::fixtures::BlockJson;
use super::*;

/// Whether the real-CLI E2E tests are enabled. They spawn the actual `codex
/// app-server` subprocess and spend API credit, so they stay off unless asked
/// for.
fn skip_unless_e2e() -> bool {
    if std::env::var("CODEX_SDK_E2E").as_deref() == Ok("1") {
        return false;
    }
    eprintln!("skipping: set CODEX_SDK_E2E=1 to run");
    true
}

/// Config for a real-CLI E2E run: a throwaway workspace named `workspace_name`.
///
/// CLI path, credentials home, and model come from the environment, so the
/// subprocess authenticates instead of hanging on an interactive login against
/// the throwaway test home.
fn e2e_config(workspace_name: &str) -> CodexConfig {
    let mut config = crate::config::test_support::minimal_config();
    let workspace = std::env::temp_dir().join(workspace_name);
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    config.workspace_dir = workspace.to_string_lossy().into_owned();
    config.repo_dir = config.workspace_dir.clone();
    if let Ok(path) = std::env::var("SANDBOX_CODEX_EXECUTABLE_PATH") {
        config.codex_cli_path = path;
    }
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        config.codex_home_dir = codex_home;
    }
    if let Ok(home) = std::env::var("HOME") {
        config.home_dir = home;
    }
    if let Ok(model) = std::env::var("DEFAULT_CODEX_MODEL") {
        config.default_codex_model = Some(model);
    }

    CodexConfig {
        config: Arc::new(config),
        credential_failures: CredentialFailureDetector::default(),
        trajectory: None,
    }
}

/// Focused real-CLI E2E: launches `codex app-server` through the production
/// [`HarnessRuntime::codex`] seam and asserts signal ordering for a trivial
/// Turn. Gated behind `CODEX_SDK_E2E=1` and `#[ignore]`.
#[tokio::test]
#[ignore = "requires the real codex CLI and credentials (CODEX_SDK_E2E=1)"]
async fn e2e_codex_turn_through_the_harness() {
    if skip_unless_e2e() {
        return;
    }

    let mut harness = HarnessRuntime::codex(e2e_config("harness-codex-e2e"));
    harness
        .submit(Input::text(
            "in-1",
            "Run the shell command `echo hello-from-codex` and then tell me its output.",
        ))
        .await
        .expect("submit input");

    let (mut started, mut accepted) = (false, false);
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let mut outcome = None;
    while let Some(signal) = harness.next_signal().await {
        match signal {
            Signal::ExecutionStarted { .. } => started = true,
            Signal::InputAccepted { .. } => accepted = true,
            Signal::Source(event) => {
                eprintln!(
                    "SOURCE type={} name={}",
                    event.block_json()["type"].as_str().unwrap_or("?"),
                    event.block_json()["name"].as_str().unwrap_or("-"),
                );
                blocks.push(event.block_json());
            }
            Signal::ExecutionFinished { outcome: finished } => {
                outcome = Some(finished);
                break;
            }
            _ => {}
        }
    }

    assert!(started, "expected ExecutionStarted");
    assert!(accepted, "expected InputAccepted");
    assert!(!blocks.is_empty(), "expected at least one source block");
    assert!(
        matches!(outcome, Some(Outcome::Completed { .. })),
        "expected Completed, got {outcome:?}"
    );

    // Every emitted block must be a canonical Anthropic-shaped block the UI can
    // render — never a raw native Codex type.
    let types: std::collections::HashSet<&str> =
        blocks.iter().filter_map(|b| b["type"].as_str()).collect();
    eprintln!("DISTINCT BLOCK TYPES: {types:?}");
    for raw in [
        "agentMessage",
        "commandExecution",
        "reasoning",
        "fileChange",
        // The exec transport's spellings, kept so the assertion still catches a
        // leak if anything reintroduces them.
        "agent_message",
        "command_execution",
        "file_change",
    ] {
        assert!(!types.contains(raw), "raw native block type leaked: {raw}");
    }
    for b in &blocks {
        let ty = b["type"].as_str().unwrap_or("");
        assert!(
            matches!(ty, "text" | "tool_use" | "tool_result" | "turn_result"),
            "non-canonical block type: {ty}"
        );
    }
    assert!(
        blocks.iter().any(|b| b["type"] == "text"),
        "expected at least one assistant text block"
    );
}

/// Everything one execution reported, folded out of the signal stream.
#[derive(Default)]
struct TurnLog {
    /// Thread id announced by `ExecutionStarted`.
    session: Option<String>,
    /// The input named as the execution's cause (`None` for a harness-internal
    /// one).
    cause: Option<InputId>,
    /// Every input accepted while this execution was open, in order. More than
    /// one would mean an input joined the running Turn.
    accepted: Vec<InputId>,
    block_types: Vec<String>,
    /// Whether the Turn had already committed a `tool_use` block when the
    /// mid-Turn message was submitted — the evidence that it was busy working,
    /// not idling at its start.
    tool_use_before_mid_turn: bool,
    /// All assistant/user text emitted by the Turn, newline-joined.
    text: String,
    outcome: Option<Outcome>,
}

/// Drain signals until the open execution reports its terminal, folding them
/// into a [`TurnLog`].
///
/// When `mid_turn` is `Some((id, text))`, it submits that input the moment the
/// execution commits its first `tool_use` block, resuming the thread the
/// running Turn announced — what `CoordinatorService::build_input` does for a
/// follow-up Turn.
async fn collect_turn(harness: &mut HarnessRuntime, mid_turn: Option<(&str, &str)>) -> TurnLog {
    let mut log = TurnLog::default();
    let mut mid_turn = mid_turn;

    loop {
        let Some(signal) = harness.next_signal().await else {
            break;
        };
        match signal {
            Signal::ExecutionStarted { session, cause, .. } => {
                eprintln!("EXECUTION STARTED session={} cause={cause:?}", session.0);
                log.session = Some(session.0);
                if let Cause::Input(id) = cause {
                    log.cause = Some(id);
                }
            }
            Signal::InputAccepted { input } => {
                eprintln!("INPUT ACCEPTED {}", input.0);
                log.accepted.push(input);
            }
            Signal::Source(event) => {
                let block = event.block_json();
                log.block_types
                    .push(block["type"].as_str().unwrap_or("?").to_string());
                if let Some(text) = block["text"].as_str() {
                    log.text.push_str(text);
                    log.text.push('\n');
                }

                if block["type"] == "tool_use" {
                    if let Some((id, text)) = mid_turn.take() {
                        eprintln!("SUBMITTING {id} while the Turn is running");
                        log.tool_use_before_mid_turn = true;
                        let input = Input {
                            resume: log.session.clone().map(ResumeSession::WithId),
                            ..Input::text(id, text)
                        };
                        harness.submit(input).await.expect("submit mid-Turn input");
                    }
                }
            }
            Signal::Activity(_) => {}
            Signal::ContextWindow(window) => {
                eprintln!("CONTEXT WINDOW {window:?}");
            }
            Signal::ExecutionFinished { outcome } => {
                eprintln!("EXECUTION FINISHED {outcome:?}");
                log.outcome = Some(outcome);
                break;
            }
            Signal::InputRejected { input, reason } => {
                panic!("input {} was rejected: {reason:?}", input.0)
            }
        }
    }

    log
}

/// Real-CLI E2E: a message sent while a Codex Turn is running is answered *in
/// that Turn*.
///
/// Submitted the moment the Turn commits a `tool_use` block, so it demonstrably
/// lands mid-work; `turn/steer` folds it in — one execution, two acceptances,
/// no second Turn, and the answer reflects the message.
#[tokio::test]
#[ignore = "requires the real codex CLI and credentials (CODEX_SDK_E2E=1)"]
async fn e2e_message_sent_mid_turn_is_answered_inside_the_running_codex_turn() {
    if skip_unless_e2e() {
        return;
    }

    let result = timeout(Duration::from_mins(6), async {
        let mut harness = HarnessRuntime::codex(e2e_config("harness-codex-mid-turn-e2e"));

        // Two separate commands keep the Turn demonstrably busy: the first
        // completes early so the test can submit while tool work is still in
        // flight, and the sleep holds the Turn open long enough for it to land.
        harness
            .submit(Input::text(
                "in-1",
                "Run two separate shell commands, not combined into one: first \
                 `echo step-one`, then `sleep 25`. After both have finished, reply \
                 with the single word ALPHA and nothing else.",
            ))
            .await
            .expect("submit the opening input");

        let turn = collect_turn(
            &mut harness,
            Some((
                "in-2",
                "Change of plan: reply with the single word BANANA and nothing else.",
            )),
        )
        .await;

        assert_eq!(
            turn.cause,
            Some(InputId("in-1".to_string())),
            "the Turn must be opened by the first input"
        );
        assert!(
            turn.tool_use_before_mid_turn,
            "the message was supposed to arrive while the Turn was doing tool work, \
             got blocks: {:?}",
            turn.block_types
        );
        assert_eq!(
            turn.accepted,
            vec![InputId("in-1".to_string()), InputId("in-2".to_string())],
            "the mid-Turn message must be accepted against the Turn already running"
        );
        assert!(
            matches!(turn.outcome, Some(Outcome::Completed { .. })),
            "the steered Turn must still finish normally"
        );
        assert!(
            turn.text.to_uppercase().contains("BANANA"),
            "the running Turn must act on the message that arrived mid-Turn, got: {}",
            turn.text
        );

        // No second execution: the mid-Turn message opened no Turn of its own.
        // Nothing more is submitted, so the harness has nothing left to run and
        // any further execution would be one this change was meant to remove.
        let extra = timeout(Duration::from_secs(20), harness.next_signal()).await;
        assert!(
            matches!(extra, Err(_) | Ok(None)),
            "the steered message must not open a second execution, got {extra:?}"
        );
    })
    .await;

    result.expect("test timed out");
}

/// Real-CLI E2E: the other half of the contract. A message sent when no Turn is
/// running still opens a Turn of its own, on the Session the previous one used
/// — steering changed what happens *inside* a Turn, not what happens between
/// them.
#[tokio::test]
#[ignore = "requires the real codex CLI and credentials (CODEX_SDK_E2E=1)"]
async fn e2e_message_sent_between_turns_opens_a_second_codex_turn() {
    if skip_unless_e2e() {
        return;
    }

    let result = timeout(Duration::from_mins(6), async {
        let mut harness = HarnessRuntime::codex(e2e_config("harness-codex-between-turns-e2e"));

        harness
            .submit(Input::text(
                "in-1",
                "Reply with the single word ALPHA and nothing else.",
            ))
            .await
            .expect("submit the opening input");
        let first = collect_turn(&mut harness, None).await;

        assert!(matches!(first.outcome, Some(Outcome::Completed { .. })));
        assert_eq!(first.accepted, vec![InputId("in-1".to_string())]);
        let session = first
            .session
            .clone()
            .expect("the first Turn announced a thread");

        // The first Turn has reported its terminal, so this one arrives between
        // Turns and resumes the thread it announced.
        harness
            .submit(Input {
                resume: Some(ResumeSession::WithId(session.clone())),
                ..Input::text(
                    "in-2",
                    "Reply with the single word BANANA and nothing else.",
                )
            })
            .await
            .expect("submit the follow-up input");
        let second = collect_turn(&mut harness, None).await;

        assert_eq!(
            second.cause,
            Some(InputId("in-2".to_string())),
            "a message sent between Turns opens the next Turn"
        );
        assert_eq!(second.accepted, vec![InputId("in-2".to_string())]);
        assert!(matches!(second.outcome, Some(Outcome::Completed { .. })));
        assert_eq!(
            second.session,
            Some(session),
            "the second Turn must resume the same Session, not start a fresh one"
        );
        assert!(
            second.text.to_uppercase().contains("BANANA"),
            "the second Turn must answer its own message, got: {}",
            second.text
        );
    })
    .await;

    result.expect("test timed out");
}
