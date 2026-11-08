//! Rebuilding the harness when a Send names another Agent profile.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::fixtures::*;
use super::service_fixtures::*;
use super::*;

use crate::agent::coordinator::service::HarnessFactory;
use crate::agent::handle::RunAck;
use crate::agent::types::AgentStatus;

fn claude_config() -> Arc<Config> {
    let mut c = minimal_config();
    c.agent_profile = Some("claude-code".to_string());
    Arc::new(c)
}

/// A factory that counts its builds, each a harness that parks its turn.
fn counting_factory(builds: &Arc<AtomicUsize>) -> HarnessFactory {
    let builds = Arc::clone(builds);
    Box::new(move |_, _| {
        builds.fetch_add(1, Ordering::SeqCst);
        Ok(HarnessRuntime::scripted(Script::new(vec![
            Step::SessionReady(SessionId("sess-2".to_string())),
            Step::AcceptInput,
            Step::Completed,
        ])))
    })
}

fn service(
    harness: HarnessRuntime,
    config: Arc<Config>,
    agent_state: Arc<AgentState>,
    cancel: CancellationToken,
    factory: HarnessFactory,
) -> (CoordinatorService, GrpcEventPublisher) {
    let (events, _received, _cancel) = recording_publisher_with_cancel(cancel);
    let coordinator = Coordinator::new(harness, events.clone(), "task-1");
    let service = CoordinatorService::new(
        coordinator,
        agent_state,
        config,
        Arc::new(RwLock::new(HashMap::new())),
        Arc::new(RecordingBackend {
            received: Arc::new(Mutex::new(Vec::new())),
        }),
        Arc::new(RwLock::new(None)),
        Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
        Arc::new(crate::commands::CommandRegistry::for_test()),
        CredentialFailureDetector::default(),
        Arc::new(Dispatcher::new()),
        crate::workspace_sync::WorkspaceSyncTracker::new(),
    )
    .with_harness_factory(factory);
    (service, events)
}

#[tokio::test]
async fn ensure_profile_rebuilds_the_harness_only_on_a_change() {
    let cancel = CancellationToken::new();
    let agent_state = Arc::new(AgentState::new());
    agent_state
        .set_session_id("claude-session".to_string())
        .await;
    let builds = Arc::new(AtomicUsize::new(0));
    let (mut service, _events) = service(
        HarnessRuntime::scripted(Script::new(Vec::new())),
        claude_config(),
        Arc::clone(&agent_state),
        cancel.clone(),
        counting_factory(&builds),
    );

    assert!(service.ensure_profile(None).await);
    assert!(service.ensure_profile(Some("claude-code")).await);
    assert_eq!(builds.load(Ordering::SeqCst), 0);

    // Same family: the session stays resumable.
    assert!(service.ensure_profile(Some("claude-code-fable")).await);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert_eq!(
        agent_state.last_session_id().await.as_deref(),
        Some("claude-session")
    );

    assert!(service.ensure_profile(Some("codex")).await);
    assert!(service.ensure_profile(Some("codex")).await);
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    assert_eq!(agent_state.last_session_id().await, None);
    cancel.cancel();
}

/// The running turn keeps its harness; the Send's ack is dropped, so the
/// dispatcher redelivers it after the turn.
#[tokio::test]
async fn a_new_profile_waits_for_the_running_turn() {
    let cancel = CancellationToken::new();
    let agent_state = Arc::new(AgentState::new());
    let builds = Arc::new(AtomicUsize::new(0));
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::WaitForCancel,
    ]);
    let (service, events) = service(
        HarnessRuntime::scripted(script),
        claude_config(),
        Arc::clone(&agent_state),
        cancel.clone(),
        counting_factory(&builds),
    );
    let (tx, rx) = mpsc::channel::<AgentCommand>(64);
    tokio::spawn(service.run(rx, Heartbeat::new()));
    let (ack_tx, _first_ack) = oneshot::channel();
    tx.try_send(send("go", None, ack_tx)).expect("channel open");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while agent_state.status().await != AgentStatus::Running {
        assert!(tokio::time::Instant::now() < deadline, "turn never ran");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let (ack_tx, ack_rx) = oneshot::channel();
    tx.try_send(send("switch", Some("codex"), ack_tx))
        .expect("channel open");
    assert!(ack_rx.await.is_err(), "the ack is dropped, not resolved");
    assert_eq!(builds.load(Ordering::SeqCst), 0);
    drop(events);
    cancel.cancel();
}

/// A fallback's Send names no model: the one the other harness's turn ran
/// must not carry over, or the new harness asks for a model it cannot serve.
#[tokio::test]
async fn a_harness_switch_drops_the_other_harness_model() {
    for (from, model, to) in [
        ("codex", "gpt-6.1-sol", "claude-code"),
        ("claude-code", "claude-opus-5-5", "codex"),
    ] {
        let cancel = CancellationToken::new();
        let agent_state = Arc::new(AgentState::new());
        let recorder: Arc<tokio::sync::Mutex<Vec<Input>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let completes = || {
            Script::new(vec![
                Step::SessionReady(SessionId("sess".to_string())),
                Step::AcceptInput,
                Step::Completed,
            ])
        };
        let factory_recorder = Arc::clone(&recorder);
        let factory: HarnessFactory = Box::new(move |_, _| {
            Ok(HarnessRuntime::scripted_recording(
                completes(),
                Arc::clone(&factory_recorder),
            ))
        });
        let mut config = minimal_config();
        config.agent_profile = Some(from.to_string());
        let (service, events) = service(
            HarnessRuntime::scripted_recording(completes(), Arc::clone(&recorder)),
            Arc::new(config),
            Arc::clone(&agent_state),
            cancel.clone(),
            factory,
        );
        let (tx, rx) = mpsc::channel::<AgentCommand>(64);
        tokio::spawn(service.run(rx, Heartbeat::new()));

        let (ack_tx, first_ack) = oneshot::channel();
        tx.try_send(send_on("go", Some(model), None, ack_tx))
            .expect("channel open");
        first_ack.await.expect("the first turn is acked");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while agent_state.status().await != AgentStatus::Idle {
            assert!(tokio::time::Instant::now() < deadline, "turn never ended");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let (ack_tx, _second_ack) = oneshot::channel();
        tx.try_send(send_on("fallback", None, Some(to), ack_tx))
            .expect("channel open");
        let second = loop {
            if let Some(input) = recorder.lock().await.get(1) {
                break input.clone();
            }
            assert!(tokio::time::Instant::now() < deadline, "no fallback input");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(second.model, None, "{from} -> {to} kept {model}");
        drop(events);
        cancel.cancel();
    }
}

fn send(text: &str, agent_profile: Option<&str>, ack: oneshot::Sender<RunAck>) -> AgentCommand {
    send_on(text, None, agent_profile, ack)
}

fn send_on(
    text: &str,
    model: Option<&str>,
    agent_profile: Option<&str>,
    ack: oneshot::Sender<RunAck>,
) -> AgentCommand {
    AgentCommand::Send {
        text: text.to_string(),
        model: model.map(str::to_string),
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        agent_profile: agent_profile.map(str::to_string),
        switch_harness: false,
        ack,
    }
}
