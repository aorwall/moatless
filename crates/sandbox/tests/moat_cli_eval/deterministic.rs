//! Scenario validation, invocation matching, report redaction, aggregation, and
//! the shim's own delegate/reject behaviour.

use super::*;

fn fixture_scenario() -> Scenario {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/moat_cli_eval/scenarios/task-create.json");
    let raw = fs::read_to_string(&path).expect("read fixture scenario");
    let mut scenario: Scenario = serde_json::from_str(&raw).expect("parse fixture scenario");
    scenario.source_path = path;
    scenario
}

#[test]
fn scenario_validation_requires_a_safe_response_for_each_expectation() {
    let mut scenario = fixture_scenario();
    scenario.mock_responses.remove("tasks create");
    let error = validate_scenario(&scenario).expect_err("scenario should be rejected");
    assert!(
        error.contains("has no safe mock response"),
        "validation should name the missing safe response: {error}"
    );
}

#[test]
fn command_matching_ignores_global_flags_but_never_counts_help() {
    let expectation = &fixture_scenario().expectations[0];
    assert!(invocation_matches(
        expectation,
        &Invocation {
            argv: vec![
                "--json".to_string(),
                "tasks".to_string(),
                "--verbose".to_string(),
                "create".to_string(),
                "--agent-profile=codex".to_string(),
            ],
        }
    ));
    assert!(!invocation_matches(
        expectation,
        &Invocation {
            argv: vec![
                "tasks".to_string(),
                "create".to_string(),
                "--help".to_string(),
            ],
        }
    ));
    assert!(invocation_matches(
        expectation,
        &Invocation {
            argv: vec![
                "tasks".to_string(),
                "create".to_string(),
                "--title".to_string(),
                "help".to_string(),
            ],
        }
    ));
}

#[test]
fn required_and_forbidden_arguments_are_enforced() {
    let expectation = CommandExpectation {
        command: vec!["tasks".to_string(), "create".to_string()],
        min_invocations: 1,
        required_args: vec!["--agent-profile".to_string()],
        forbidden_args: vec!["--headless=false".to_string()],
    };
    assert!(invocation_matches(
        &expectation,
        &Invocation {
            argv: vec![
                "tasks".to_string(),
                "create".to_string(),
                "--agent-profile=codex".to_string(),
            ],
        }
    ));
    assert!(!invocation_matches(
        &expectation,
        &Invocation {
            argv: vec![
                "tasks".to_string(),
                "create".to_string(),
                "--agent-profile=codex".to_string(),
                "--headless=false".to_string(),
            ],
        }
    ));
}

#[test]
fn report_redaction_covers_inline_and_following_secret_values() {
    assert_eq!(
        redact_argv(&[
            "secret".to_string(),
            "set".to_string(),
            "--token".to_string(),
            "plain-token".to_string(),
            "--api-key=plain-key".to_string(),
        ]),
        vec![
            "secret",
            "set",
            "--token",
            "<redacted>",
            "--api-key=<redacted>",
        ]
    );
}

#[tokio::test]
async fn timed_out_trial_keeps_its_redacted_trajectory() {
    use moatless_sandbox::agent::harness::content::CanonicalContentBlock;

    let secret = "sk-eval-secret-that-must-not-leak";
    let started = Instant::now();
    let mut recorder = TrajectoryRecorder::new(started, ReportRedactor::new([secret]));
    let block = CanonicalContentBlock::ToolUse {
        id: "tool-1".to_string(),
        name: "Bash".to_string(),
        input: serde_json::json!({
            "command": format!("moat tasks create --token {secret}")
        }),
    };

    let timed = tokio::time::timeout(Duration::from_millis(1), async {
        assert!(recorder.observe(Signal::Source(block)).is_none());
        std::future::pending::<()>().await;
    })
    .await;
    assert!(timed.is_err());
    recorder.record_timeout(Duration::from_millis(1));

    // A timeout never reaches the `ExecutionFinished` terminal that resolves
    // a model from `RunTotals`.
    assert!(recorder.resolved_model.is_none());
    let trajectory = serde_json::to_value(&recorder.events).expect("serialize trajectory");
    let serialized = trajectory.to_string();
    assert!(serialized.contains("tool_use"));
    assert!(serialized.contains("timed_out"));
    assert!(serialized.contains("elapsedMs"));
    assert!(!serialized.contains("elapsed_ms"));
    assert!(serialized.contains("<redacted>"));
    assert!(!serialized.contains(secret));
}

#[test]
fn aggregation_requires_the_threshold_for_each_agent() {
    let scenario = fixture_scenario();
    let trials = [true, true, false]
        .into_iter()
        .enumerate()
        .map(|(index, passed)| TrialReport {
            scenario_id: scenario.id.clone(),
            prompt: scenario.prompt.clone(),
            system_instruction: scenario.system_instruction.clone(),
            agent: EvalAgent::ClaudeCode,
            repetition: index + 1,
            requested_model: None,
            resolved_model: None,
            duration_ms: 1,
            terminal_outcome: "completed".to_string(),
            trajectory: Vec::new(),
            invocations: Vec::new(),
            expectation_counts: BTreeMap::new(),
            passed,
        })
        .collect::<Vec<_>>();
    let summaries = summarize(&[scenario], &[EvalAgent::ClaudeCode], &trials, 2);
    assert_eq!(
        summaries,
        vec![AgentSummary {
            scenario_id: "MOAT-CLI-001".to_string(),
            agent: EvalAgent::ClaudeCode,
            passes: 2,
            trials: 3,
            required_passes: 2,
            passed: true,
        }]
    );
}

#[test]
fn mock_moat_delegates_help_and_rejects_unknown_commands() {
    let tmp = TempDir::new().expect("create mock test dir");
    let fake_real = tmp.path().join("moat-real");
    fs::write(&fake_real, "#!/bin/sh\necho delegated-help\n").expect("write fake real moat");
    let mut permissions = fs::metadata(&fake_real)
        .expect("inspect fake moat")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_real, permissions).expect("make fake moat executable");

    let scenario = fixture_scenario();
    let log = tmp.path().join("calls.jsonl");
    let shim = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/moat_cli_eval/mock_moat.sh");
    let help = StdCommand::new("sh")
        .arg(&shim)
        .args(["tasks", "create", "--help"])
        .env("MOAT_CLI_EVAL_LOG", &log)
        .env("MOAT_CLI_REAL_BIN", &fake_real)
        .env("MOAT_CLI_EVAL_SCENARIO_FILE", &scenario.source_path)
        .output()
        .expect("run help through mock moat");
    assert!(help.status.success(), "delegated help should succeed");
    assert_eq!(
        String::from_utf8_lossy(&help.stdout).trim(),
        "delegated-help"
    );

    let unknown = StdCommand::new("sh")
        .arg(&shim)
        .args(["teams", "delete"])
        .env("MOAT_CLI_EVAL_LOG", &log)
        .env("MOAT_CLI_REAL_BIN", &fake_real)
        .env("MOAT_CLI_EVAL_SCENARIO_FILE", &scenario.source_path)
        .output()
        .expect("run unknown command through mock moat");
    assert_eq!(unknown.status.code(), Some(64));
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("no safe response configured"),
        "unknown command should explain that it was rejected"
    );

    let help_as_value = StdCommand::new("sh")
        .arg(&shim)
        .args(["tasks", "create", "--title", "help"])
        .env("MOAT_CLI_EVAL_LOG", &log)
        .env("MOAT_CLI_REAL_BIN", &fake_real)
        .env("MOAT_CLI_EVAL_SCENARIO_FILE", &scenario.source_path)
        .output()
        .expect("run a safe command containing help as an argument value");
    assert!(help_as_value.status.success());
    assert!(
        !String::from_utf8_lossy(&help_as_value.stdout).contains("delegated-help"),
        "an argument value named help must not bypass the safe response"
    );

    let calls = read_invocations(&log).expect("read mock invocation log");
    assert_eq!(
        calls.len(),
        3,
        "delegated, rejected, and mocked calls should all be recorded"
    );
}
