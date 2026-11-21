use std::collections::HashMap;

use super::*;
use crate::agent::harness::signal::InputId;
use crate::agent::harness::trajectory::SessionFormat;

/// A sandbox home, workspace and runtime dir with the fake agent on `PATH`.
struct Fixture {
    dir: tempfile::TempDir,
    config: Arc<crate::config::Config>,
}

impl Fixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let agy = bin.join("agy");
        std::fs::write(&agy, include_str!("fake_agy.py")).unwrap();
        std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o755)).unwrap();
        for sub in ["home", "workspace", "runtime"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        std::fs::write(dir.path().join("runtime/.user_auth_token"), "task-token\n").unwrap();
        let mut config = crate::config::test_support::minimal_config();
        config.home_dir = dir.path().join("home").display().to_string();
        config.workspace_dir = dir.path().join("workspace").display().to_string();
        config.runtime_dir = dir.path().join("runtime").display().to_string();
        config.backend_api_url = Some("http://127.0.0.1:9".into());
        Self {
            dir,
            config: Arc::new(config),
        }
    }

    fn process(&self) -> AcpProcess {
        AcpProcess::new(AcpConfig {
            config: Arc::clone(&self.config),
            trajectory: None,
            profile: moatless_types::agent::AgentProfile::Antigravity,
        })
    }

    fn input(&self, id: &str, text: &str) -> Input {
        let path = format!(
            "{}:{}",
            self.dir.path().join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut input = Input::text(id, text);
        input.model = Some("gemini-3-flash".into());
        input.env = Some(HashMap::from([
            ("PATH".to_string(), path),
            (
                "FAKE_ACP_LOG".to_string(),
                self.dir.path().join("requests.jsonl").display().to_string(),
            ),
            ("OPENAI_API_KEY".to_string(), "must-not-leak".to_string()),
        ]));
        input
    }

    fn requests(&self, method: &str) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|v| v["method"] == method)
            .collect()
    }

    fn journal(&self, id: &str) -> String {
        std::fs::read_to_string(super::super::journal::journal_path(
            &self.config.acp_home_dir(),
            id,
        ))
        .unwrap()
    }
}

/// Events until the execution's terminal, inclusive.
async fn run_to_end(process: &mut AcpProcess) -> Vec<ProcessEvent> {
    let mut events = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(20), process.next())
            .await
            .expect("the execution ends")
            .expect("a persistent process never closes");
        let end = matches!(event, ProcessEvent::ExecutionEnded { .. });
        events.push(event);
        if end {
            return events;
        }
    }
}

fn session_of(events: &[ProcessEvent]) -> String {
    events
        .iter()
        .find_map(|e| match e {
            ProcessEvent::SessionReady { session, .. } => Some(session.0.clone()),
            _ => None,
        })
        .expect("session announced")
}

fn completed(events: &[ProcessEvent]) -> bool {
    matches!(
        events.last(),
        Some(ProcessEvent::ExecutionEnded {
            termination: ProcessTermination::Completed
        })
    )
}

#[tokio::test]
async fn a_turn_streams_blocks_and_totals_and_journals_the_conversation() {
    let fx = Fixture::new();
    let mut acp = fx.process();
    acp.submit(fx.input("m1", "remember 42")).await.unwrap();

    let events = run_to_end(&mut acp).await;

    assert!(matches!(events[0], ProcessEvent::SessionReady { .. }));
    assert!(matches!(&events[1], ProcessEvent::InputAccepted { input } if input.0 == "m1"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ProcessEvent::Block(Block::ToolUse { id, .. }) if id == "call-1"))
    );
    assert!(events.iter().any(
        |e| matches!(e, ProcessEvent::Block(Block::Text { text }) if text == "echo: remember 42")
    ));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ProcessEvent::ContextWindow(w) if w.used_tokens == 1234))
    );
    let totals = events.iter().find_map(|e| match e {
        ProcessEvent::RunTotals(t) => Some(t.clone()),
        _ => None,
    });
    let totals = totals.expect("totals before the terminal");
    assert_eq!(
        (totals.input_tokens, totals.cache_read_input_tokens),
        (10, 100)
    );
    assert!(completed(&events));

    let journal = fx.journal(&session_of(&events));
    let kinds: Vec<String> = journal
        .lines()
        .map(|l| AcpSessionFormat.read_line(l).kind)
        .collect();
    assert_eq!(
        kinds,
        [
            "session",
            "binding",
            "user",
            "thought",
            "tool_call",
            "tool_result",
            "assistant",
            "turn_end"
        ]
    );
    let model = &fx.requests("session/set_config_option")[0]["params"];
    assert_eq!(model["value"], "gemini-3-flash");
}

#[tokio::test]
async fn a_later_turn_reuses_the_agent_and_a_new_process_loads_its_session() {
    let fx = Fixture::new();
    let mut acp = fx.process();
    acp.submit(fx.input("m1", "first")).await.unwrap();
    let session = session_of(&run_to_end(&mut acp).await);
    // A new driver rotates the task token; the gateway proxy reads it per call.
    std::fs::write(fx.dir.path().join("runtime/.user_auth_token"), "rotated\n").unwrap();
    let mut second = fx.input("m2", "second");
    second.resume = Some(ResumeSession::WithId(session.clone()));
    acp.submit(second.clone()).await.unwrap();
    run_to_end(&mut acp).await;
    assert_eq!(fx.requests("initialize").len(), 1);
    drop(acp);

    let mut replaced = fx.process();
    second.id = InputId("m3".into());
    second.text = "third".into();
    replaced.submit(second).await.unwrap();
    let events = run_to_end(&mut replaced).await;

    assert_eq!(session_of(&events), session);
    assert_eq!(fx.requests("session/load").len(), 1);
    assert_eq!(fx.requests("session/new").len(), 1);
    let prompts = fx.requests("session/prompt");
    assert_eq!(
        prompts.last().unwrap()["params"]["prompt"][0]["text"],
        "third"
    );
    assert!(!fx.journal(&session).contains("replayed"));
}

#[tokio::test]
async fn a_journal_whose_agent_session_is_gone_is_reseeded_into_a_new_one() {
    let fx = Fixture::new();
    let mut acp = fx.process();
    acp.submit(fx.input("m1", "the code is 42")).await.unwrap();
    let session = session_of(&run_to_end(&mut acp).await);
    drop(acp);
    std::fs::remove_dir_all(fx.config.antigravity_dir().join("antigravity-cli")).unwrap();

    let mut replaced = fx.process();
    let mut next = fx.input("m2", "what was the code?");
    next.resume = Some(ResumeSession::WithId(session));
    replaced.submit(next).await.unwrap();
    assert!(completed(&run_to_end(&mut replaced).await));

    let prompt = fx.requests("session/prompt").last().unwrap()["params"]["prompt"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(prompt.starts_with("<conversation_history>"), "{prompt}");
    assert!(prompt.contains("the code is 42") && prompt.ends_with("what was the code?"));
    assert_eq!(fx.requests("session/new").len(), 2);
}

#[tokio::test]
async fn steering_joins_the_running_execution() {
    let fx = Fixture::new();
    let mut acp = fx.process();
    acp.submit(fx.input("m1", "SLOW work")).await.unwrap();
    assert!(matches!(
        acp.next().await,
        Some(ProcessEvent::SessionReady { .. })
    ));
    assert!(matches!(
        acp.next().await,
        Some(ProcessEvent::InputAccepted { .. })
    ));

    acp.submit(fx.input("m2", "also this")).await.unwrap();
    acp.cancel().await;
    let events = run_to_end(&mut acp).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, ProcessEvent::InputAccepted { input } if input.0 == "m2"))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ProcessEvent::SessionReady { .. }))
    );
    assert_eq!(fx.requests("session/cancel").len(), 1);
}

#[tokio::test]
async fn a_wake_up_after_the_turn_runs_as_an_internal_execution() {
    let fx = Fixture::new();
    let mut acp = fx.process();
    acp.submit(fx.input("m1", "BACKGROUND job")).await.unwrap();
    run_to_end(&mut acp).await;

    let events = run_to_end(&mut acp).await;

    assert!(matches!(events[0], ProcessEvent::InternalStart { .. }));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ProcessEvent::Block(Block::Text { text }) if text == "woke up"))
    );
    assert!(completed(&events));
}

#[tokio::test]
async fn a_prompt_that_returns_while_its_tool_runs_ends_after_the_wake_up() {
    let fx = Fixture::new();
    let mut acp = fx.process();
    acp.submit(fx.input("m1", "BGTOOL job")).await.unwrap();
    let mut events = Vec::new();
    while !events
        .iter()
        .any(|e| matches!(e, ProcessEvent::Block(Block::ToolUse { .. })))
    {
        events.push(acp.next().await.unwrap());
    }
    acp.submit(fx.input("m2", "steer")).await.unwrap();
    events.extend(run_to_end(&mut acp).await);

    let text = |want: &str| {
        events
            .iter()
            .any(|e| matches!(e, ProcessEvent::Block(Block::Text { text }) if text.contains(want)))
    };
    assert!(text("bg done") && text("steer"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ProcessEvent::InputAccepted { input } if input.0 == "m2"))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ProcessEvent::InternalStart { .. }))
    );
    assert!(completed(&events));
}

#[tokio::test]
async fn permission_is_granted_and_an_unknown_model_fails_before_the_turn() {
    let fx = Fixture::new();
    let mut acp = fx.process();
    acp.submit(fx.input("m1", "PERMISSION please"))
        .await
        .unwrap();
    run_to_end(&mut acp).await;
    // The fake logs our answer on its own thread, possibly after the turn ends.
    let mut answer = None;
    for _ in 0..50 {
        answer = std::fs::read_to_string(fx.dir.path().join("requests.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|v| v["id"] == "perm-1");
        if answer.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let answer = answer.expect("permission answered");
    assert_eq!(answer["result"]["outcome"]["optionId"], "yes");

    let mut bad = fx.input("m2", "hi");
    bad.model = Some("unknown-model".into());
    let err = acp.submit(bad).await.expect_err("model refused");
    assert_eq!(err.class, ErrorClass::InvalidConfiguration);
}
