//! Submitting and sending through the service.

use super::fixtures::*;
use super::service_fixtures::*;
use super::*;

/// Stamped per Member; see `crates/sandbox/AGENTS.md` "Spawning a child
/// process".
#[tokio::test]
async fn coordinator_service_submits_this_members_task_id_to_the_harness() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.task_id = "fork-task".to_string();
        c.agent_profile = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());
    let recorder: Arc<tokio::sync::Mutex<Vec<Input>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Completed,
    ]);
    let (tx, _received, _events) = service_with_harness(
        HarnessRuntime::scripted_recording(script, Arc::clone(&recorder)),
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.try_send(AgentCommand::Send {
        text: "do the thing".to_string(),
        model: None,
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        agent_profile: None,
        switch_harness: false,
        ack: ack_tx,
    })
    .expect("channel open");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let env = loop {
        if let Some(input) = recorder.lock().await.first() {
            break input.env.clone().expect("input carries an environment");
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no input reached the harness within 5 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    assert_eq!(env.get("TASK_ID").map(String::as_str), Some("fork-task"));
    cancel.cancel();
}

/// Drive a Submit through a successful scripted execution and assert that
/// `agent_state.status()` ends `Idle` and a `completed` event was published.
#[tokio::test]
async fn coordinator_service_submit_completed_sets_idle_and_publishes_completed() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block(text_block("response"))),
        Step::Completed,
    ]);
    let (tx, received, events) = service_with_script(
        script,
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.try_send(AgentCommand::Send {
        text: "do the thing".to_string(),
        model: None,
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        agent_profile: None,
        switch_harness: false,
        ack: ack_tx,
    })
    .expect("channel open");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        flush(&events).await;
        let types = sub_types(&received);
        if types.contains(&"completed".to_string()) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "completed event not published within 5 s; got {types:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    cancel.cancel();
}

#[tokio::test]
async fn coordinator_service_sends_during_active_turn_into_running_harness() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    let script = Script::injecting(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::AwaitInput,
        Step::AcceptInput,
        Step::Block(block(text_block("saw the merge conflict"))),
        Step::Completed,
    ]);
    let (tx, received, events) = service_with_harness(
        HarnessRuntime::scripted(script),
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    let (first_ack, first_rx) = oneshot::channel();
    tx.try_send(AgentCommand::Send {
        text: "open pr".to_string(),
        model: None,
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        agent_profile: None,
        switch_harness: false,
        ack: first_ack,
    })
    .expect("channel open");
    let first = tokio::time::timeout(Duration::from_secs(5), first_rx)
        .await
        .expect("first input accepted")
        .expect("first ack delivered");

    let (second_ack, second_rx) = oneshot::channel();
    tx.try_send(AgentCommand::Send {
        text: "merge conflict notice".to_string(),
        model: None,
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        agent_profile: None,
        switch_harness: false,
        ack: second_ack,
    })
    .expect("channel open");

    let second = tokio::time::timeout(Duration::from_secs(5), second_rx)
        .await
        .expect("injected input accepted and followed by output")
        .expect("second ack delivered");
    flush(&events).await;

    assert_eq!(first.session_id, "sess-1");
    assert_eq!(second.session_id, "sess-1");
    assert_eq!(first.run_id, second.run_id);
    assert_eq!(
        sub_types(&received)
            .into_iter()
            .filter(|sub_type| sub_type == "started")
            .count(),
        1,
        "the second send must join the existing run, not open a deferred one"
    );

    cancel.cancel();
}

/// A synthetic scheduled-job Send carries no explicit model. It must reuse the
/// last selected model instead of falling back to the sandbox default.
#[tokio::test]
async fn coordinator_service_send_without_model_reuses_last_model() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("codex".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());
    let recorder = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-model".to_string())),
        Step::AcceptInput,
        Step::Completed,
    ]);
    let (tx, _received, events) = service_with_harness(
        HarnessRuntime::scripted_recording(script, Arc::clone(&recorder)),
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    let (first_ack, _first_rx) = oneshot::channel();
    tx.try_send(AgentCommand::Send {
        text: "first".to_string(),
        model: Some("gpt-5.6-sol".to_string()),
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        agent_profile: None,
        switch_harness: false,
        ack: first_ack,
    })
    .expect("channel open");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        flush(&events).await;
        if !recorder.lock().await.is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "first input not recorded within 5 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let (second_ack, _second_rx) = oneshot::channel();
    tx.try_send(AgentCommand::Send {
        text: "synthetic".to_string(),
        model: None,
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        agent_profile: None,
        switch_harness: false,
        ack: second_ack,
    })
    .expect("channel open");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        flush(&events).await;
        if recorder.lock().await.len() >= 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "second input not recorded within 5 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let inputs = recorder.lock().await;
    assert_eq!(inputs[0].model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(
        inputs[1].model.as_deref(),
        Some("gpt-5.6-sol"),
        "synthetic sends with model=None must preserve the selected model"
    );

    cancel.cancel();
}

#[tokio::test]
async fn coordinator_service_recover_replays_the_interrupted_turns_model() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("claude-code".to_string());
        Arc::new(c)
    };
    let recorder = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-recover".to_string())),
        Step::AcceptInput,
        Step::Completed,
    ]);
    let (tx, _received, events) = service_with_harness(
        HarnessRuntime::scripted_recording(script, Arc::clone(&recorder)),
        config,
        Arc::new(AgentState::new()),
        cancel.clone(),
    );

    tx.try_send(AgentCommand::Recover {
        text: "continue".to_string(),
        agent_mode: None,
        effort: None,
        model: Some("claude-haiku-4-5".to_string()),
        resume_session_id: "sess-recover".to_string(),
        run_id: None,
    })
    .expect("channel open");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        flush(&events).await;
        if let Some(input) = recorder.lock().await.first() {
            assert_eq!(input.model.as_deref(), Some("claude-haiku-4-5"));
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "recovered input not recorded within 5 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cancel.cancel();
}

/// Run one scripted turn to `completed` and return that event's payload.
async fn completed_payload(commands: Arc<crate::commands::CommandRegistry>) -> serde_json::Value {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("claude-code".to_string());
        Arc::new(c)
    };
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Completed,
    ]);
    let (tx, received, events) = service_with_commands(
        HarnessRuntime::scripted(script),
        config,
        Arc::new(AgentState::new()),
        cancel.clone(),
        commands,
    );
    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.try_send(AgentCommand::Send {
        text: "go".to_string(),
        model: None,
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        agent_profile: None,
        switch_harness: false,
        ack: ack_tx,
    })
    .expect("channel open");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while count_sub_type(&received, "completed") == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "completed event not published within 5 s"
        );
        flush(&events).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let payload = only(&received, "completed").payload_json;
    cancel.cancel();
    serde_json::from_str(&payload).expect("payload is JSON")
}

/// The backend holds a child's parent notice on this count; see
/// `handle_agent_completed`.
#[tokio::test]
async fn completed_counts_the_commands_the_background_watch_will_wake_for() {
    let commands = Arc::new(crate::commands::CommandRegistry::for_test());
    let running = commands
        .run(crate::commands::CommandSpec {
            command: "sleep 30".to_string(),
            cwd: None,
            timeout: None,
            label: None,
        })
        .await
        .expect("spawn");

    let payload = completed_payload(Arc::clone(&commands)).await;
    commands.kill(&running.id).await.expect("kill");

    assert_eq!(payload["backgroundCommands"], 1);
}

#[tokio::test]
async fn completed_without_background_commands_carries_no_count() {
    let payload = completed_payload(Arc::new(crate::commands::CommandRegistry::for_test())).await;

    assert!(payload.get("backgroundCommands").is_none(), "{payload}");
}
