//! Credential recovery: re-drive on something fresher, else give up once.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use super::fixtures::*;
use super::service_fixtures::*;
use super::*;

/// A backend whose `get_credentials` returns a configurable `CODEX_AUTH_JSON`
/// value, optionally rotating it so each recovery pull is genuinely fresher.
///
/// Optionally flags a shared detector on every call, standing in for Codex
/// re-emitting `refresh_token_reused`; every other call is inert.
struct CredBackend {
    /// The `CODEX_AUTH_JSON` value returned when `rotate` is false.
    base: String,
    /// When true, return a monotonically fresh value each call instead of
    /// `base`.
    rotate: bool,
    counter: AtomicU64,
    /// When set, flip this detector's Codex flag on every `get_credentials` —
    /// simulating a Turn that fails on a stale credential.
    flag: Option<CredentialFailureDetector>,
}

#[async_trait]
impl BackendClient for CredBackend {
    async fn get_credentials(&self, _task_id: &str) -> Result<GetCredentialsResponse, Status> {
        if let Some(detector) = &self.flag {
            detector.trigger(CODEX_AUTH_JSON);
        }
        let value = if self.rotate {
            let n = self.counter.fetch_add(1, Ordering::SeqCst);
            format!("{{\"tokens\":{{\"refresh_token\":\"tok-{n}\"}}}}")
        } else {
            self.base.clone()
        };
        Ok(GetCredentialsResponse {
            credentials: HashMap::from([("CODEX_AUTH_JSON".to_string(), value)]),
            ssh_keys: HashMap::new(),
        })
    }
    async fn get_system_prompt(&self, _task_id: &str) -> Result<String, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn download_upload(
        &self,
        _task_id: &str,
        _file_id: &str,
    ) -> Result<DownloadUploadResponse, Status> {
        Err(Status::unimplemented("unused"))
    }
}

/// A credential sink that writes the recovered value to a file (the same
/// `auth.json` the disk-dedup reads) and counts how many times it applied.
struct WritingSink {
    path: PathBuf,
    applied: Arc<AtomicUsize>,
}

#[async_trait]
impl CredentialSink for WritingSink {
    async fn apply(&self, value: &str) -> Result<(), String> {
        tokio::fs::write(&self.path, value)
            .await
            .map_err(|e| e.to_string())?;
        self.applied.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A temp `~/.codex` with `auth.json` seeded to `on_disk_auth`, plus a codex
/// `Config` pointed at it. The returned `TempDir` must be kept alive.
fn codex_config_with_auth(on_disk_auth: &str) -> (tempfile::TempDir, Arc<Config>, PathBuf) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_dir = tmp.path().join(".codex");
    std::fs::create_dir_all(&codex_dir).expect("mkdir codex");
    let auth_path = codex_dir.join("auth.json");
    std::fs::write(&auth_path, on_disk_auth).expect("write auth.json");
    let mut config = minimal_config();
    config.agent_profile = Some("codex".to_string());
    config.codex_home_dir = codex_dir.display().to_string();
    (tmp, Arc::new(config), auth_path)
}

/// Build a `CoordinatorService` over a scripted harness with a custom backend,
/// detector, and dispatcher — the credential-recovery collaborators.
#[allow(clippy::too_many_arguments)] // reason: test wiring mirrors the service constructor
fn service_with_credentials(
    harness: HarnessRuntime,
    config: Arc<Config>,
    agent_state: Arc<AgentState>,
    backend: Arc<dyn BackendClient>,
    detector: CredentialFailureDetector,
    dispatcher: Arc<Dispatcher>,
    cancel: CancellationToken,
) -> (
    mpsc::Sender<AgentCommand>,
    Arc<Mutex<Vec<PublishEventRequest>>>,
    GrpcEventPublisher,
) {
    let (events, received, _outbox_cancel) = recording_publisher_with_cancel(cancel);
    let coordinator = Coordinator::new(harness, events.clone(), "task-1");
    let service = CoordinatorService::new(
        coordinator,
        agent_state,
        config,
        Arc::new(RwLock::new(HashMap::new())),
        backend,
        Arc::new(RwLock::new(None)),
        Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
        Arc::new(crate::commands::CommandRegistry::for_test()),
        detector,
        dispatcher,
        crate::workspace_sync::WorkspaceSyncTracker::new(),
    );
    let (tx, rx) = mpsc::channel::<AgentCommand>(64);
    tokio::spawn(service.run(rx, Heartbeat::new()));
    (tx, received, events)
}

/// A `Send` with a throwaway ack.
fn send_text(tx: &mpsc::Sender<AgentCommand>, text: &str) {
    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.try_send(AgentCommand::Send {
        text: text.to_string(),
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
}

/// A scripted Turn that accepts its input and then fails.
fn failing_turn() -> Script {
    Script::new(vec![
        Step::SessionReady(SessionId("sess-cred".to_string())),
        Step::AcceptInput,
        Step::Failed(failure()),
    ])
}

/// Poll (flushing the outbox) until `cond` holds or the deadline elapses.
async fn wait_until<F: Fn() -> bool>(events: &GrpcEventPublisher, cond: F, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    loop {
        flush(events).await;
        if cond() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

/// The terminal reconnect errors credential recovery gives up with.
fn count_reconnect_errors(received: &Arc<Mutex<Vec<PublishEventRequest>>>) -> usize {
    received
        .lock()
        .expect("lock")
        .iter()
        .filter(|r| {
            r.sub_type == "error" && r.payload_json.contains("credential_recovery_exhausted")
        })
        .count()
}

/// A fresher backend credential re-drives the SAME prompt on the SAME Run.
///
/// The failed Turn is suppressed to a single `retrying`
/// (`credential_recovery`), the credential applied, and the re-driven Turn
/// completes under one Run id with no terminal `error`.
#[tokio::test]
async fn coordinator_service_credential_recovery_redrives_same_run_on_fresher_credential() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let (_tmp, config, auth_path) = codex_config_with_auth(r#"{"tokens":{"refresh_token":"old"}}"#);
    let agent_state = Arc::new(AgentState::new());
    let detector = CredentialFailureDetector::default();

    // The backend holds a genuinely fresher credential than what's on disk.
    let backend = Arc::new(CredBackend {
        base: r#"{"tokens":{"refresh_token":"fresh"}}"#.to_string(),
        rotate: false,
        counter: AtomicU64::new(0),
        flag: None,
    });
    let applied = Arc::new(AtomicUsize::new(0));
    let mut dispatcher = Dispatcher::new();
    dispatcher.register(
        "CODEX_AUTH_JSON",
        Arc::new(WritingSink {
            path: auth_path,
            applied: Arc::clone(&applied),
        }),
    );

    // First Turn fails on the stale credential; the recovery re-drive completes.
    let scripts = vec![
        failing_turn(),
        Script::new(vec![
            Step::SessionReady(SessionId("sess-cred".to_string())),
            Step::AcceptInput,
            Step::Block(block(text_block("recovered"))),
            Step::Completed,
        ]),
    ];
    let (tx, received, events) = service_with_credentials(
        HarnessRuntime::scripted_sequence(scripts),
        config,
        Arc::clone(&agent_state),
        backend,
        detector.clone(),
        Arc::new(dispatcher),
        cancel.clone(),
    );

    // Flag the credential failure (as Codex would on the failing Turn), then send.
    detector.trigger(CODEX_AUTH_JSON);
    send_text(&tx, "do the thing");

    wait_until(
        &events,
        || sub_types(&received).contains(&"completed".to_string()),
        "completed after credential-recovery retry",
    )
    .await;

    let types = sub_types(&received);
    assert!(
        types.contains(&"retrying".to_string()),
        "expected a retrying event; got {types:?}"
    );
    assert!(
        !types.contains(&"error".to_string()),
        "a recovered credential must not surface a terminal error; got {types:?}"
    );

    // The retrying event names credential recovery as the reason.
    let retrying: serde_json::Value =
        serde_json::from_str(&only(&received, "retrying").payload_json).expect("json");
    assert_eq!(retrying["reason"], "credential_recovery");

    // The recovered credential was applied.
    assert!(
        applied.load(Ordering::SeqCst) >= 1,
        "recovered credential should have been dispatched to its sink"
    );

    // The retry re-used the one Run id (same prompt, same Turn).
    let run_ids: std::collections::HashSet<String> = received
        .lock()
        .expect("lock")
        .iter()
        .map(|r| r.run_id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    assert_eq!(
        run_ids.len(),
        1,
        "recovery must preserve one Run; saw {run_ids:?}"
    );
    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    cancel.cancel();
}

/// An unchanged credential (the backend has nothing fresher than what's on
/// disk) produces exactly ONE terminal reconnect error and NO retry — recovery
/// detects the dedup and surfaces the user-facing error instead of burning a
/// re-drive.
#[tokio::test]
async fn coordinator_service_credential_recovery_unchanged_gives_one_terminal_error() {
    let cancel = CancellationToken::new();
    let on_disk = r#"{"tokens":{"refresh_token":"same"}}"#;
    let (_tmp, config, _auth) = codex_config_with_auth(on_disk);
    let agent_state = Arc::new(AgentState::new());
    let detector = CredentialFailureDetector::default();

    // The backend returns exactly the on-disk value: nothing fresher.
    let backend = Arc::new(CredBackend {
        base: on_disk.to_string(),
        rotate: false,
        counter: AtomicU64::new(0),
        flag: None,
    });

    let (tx, received, events) = service_with_credentials(
        HarnessRuntime::scripted(failing_turn()),
        config,
        Arc::clone(&agent_state),
        backend,
        detector.clone(),
        Arc::new(Dispatcher::new()),
        cancel.clone(),
    );

    detector.trigger(CODEX_AUTH_JSON);
    send_text(&tx, "do the thing");

    wait_until(
        &events,
        || count_reconnect_errors(&received) >= 1,
        "one terminal reconnect error",
    )
    .await;
    // Let any (wrongly) scheduled retry have a chance to surface a second event.
    tokio::time::sleep(Duration::from_millis(100)).await;
    flush(&events).await;

    let types = sub_types(&received);
    assert!(
        !types.contains(&"retrying".to_string()),
        "an unchanged credential must not schedule a futile retry; got {types:?}"
    );
    assert_eq!(
        count_reconnect_errors(&received),
        1,
        "exactly one terminal reconnect error; got {types:?}"
    );

    // The single error names the provider and the no-fresher reason.
    let error: serde_json::Value = serde_json::from_str(
        &received
            .lock()
            .expect("lock")
            .iter()
            .find(|r| r.sub_type == "error")
            .expect("error present")
            .payload_json,
    )
    .expect("json");
    assert_eq!(error["reason"], "no_fresher_credential_available");
    assert_eq!(error["provider"], "Codex");
    assert!(
        error["error"]
            .as_str()
            .unwrap_or_default()
            .contains("reconnect"),
        "reconnect message; got {error:?}"
    );
    // The reconnect message is recorded as the agent's last error.
    assert!(
        agent_state
            .last_error()
            .await
            .unwrap_or_default()
            .contains("reconnect"),
        "last_error should carry the reconnect message"
    );

    cancel.cancel();
}

/// Credential recovery is capped per prompt.
///
/// With an always-fresher backend and a Turn that keeps failing, recovery
/// re-drives up to the cap then gives up with one terminal error — exactly
/// `MAX_CREDENTIAL_RECOVERY_RETRIES` `retrying` events precede it, and the Turn
/// never completes.
#[tokio::test]
async fn coordinator_service_credential_recovery_caps_and_then_gives_up() {
    let cancel = CancellationToken::new();
    let (_tmp, config, auth_path) =
        codex_config_with_auth(r#"{"tokens":{"refresh_token":"tok-init"}}"#);
    let agent_state = Arc::new(AgentState::new());
    let detector = CredentialFailureDetector::default();

    // Every pull is fresher (so recovery always "succeeds"), and every Turn is
    // re-flagged (so the credential failure recurs) — only the per-prompt cap
    // stops the loop.
    let backend = Arc::new(CredBackend {
        base: String::new(),
        rotate: true,
        counter: AtomicU64::new(0),
        flag: Some(detector.clone()),
    });
    let applied = Arc::new(AtomicUsize::new(0));
    let mut dispatcher = Dispatcher::new();
    dispatcher.register(
        "CODEX_AUTH_JSON",
        Arc::new(WritingSink {
            path: auth_path,
            applied: Arc::clone(&applied),
        }),
    );

    // Three failing Turns: two recovery re-drives, then the cap gives up.
    let scripts = vec![failing_turn(), failing_turn(), failing_turn()];
    let (tx, received, events) = service_with_credentials(
        HarnessRuntime::scripted_sequence(scripts),
        config,
        Arc::clone(&agent_state),
        backend,
        detector.clone(),
        Arc::new(dispatcher),
        cancel.clone(),
    );

    send_text(&tx, "do the thing");

    wait_until(
        &events,
        || count_reconnect_errors(&received) >= 1,
        "terminal reconnect error after cap exhaustion",
    )
    .await;

    let types = sub_types(&received);
    let retrying = types.iter().filter(|t| *t == "retrying").count();
    assert_eq!(
        retrying, MAX_CREDENTIAL_RECOVERY_RETRIES as usize,
        "exactly the cap of credential-recovery retries precede the give-up; got {types:?}"
    );
    assert_eq!(
        count_reconnect_errors(&received),
        1,
        "one terminal error after the cap; got {types:?}"
    );
    assert!(
        !types.contains(&"completed".to_string()),
        "the Turn never completed; got {types:?}"
    );

    // The give-up names cap exhaustion.
    let error: serde_json::Value = serde_json::from_str(
        &received
            .lock()
            .expect("lock")
            .iter()
            .find(|r| r.sub_type == "error")
            .expect("error present")
            .payload_json,
    )
    .expect("json");
    assert_eq!(error["reason"], "credential_recovery_exhausted");

    cancel.cancel();
}

/// Each credential-recovery give-up counts as exactly ONE consecutive failure.
///
/// It takes three give-ups to trip the circuit breaker on the fourth Send; a
/// double-count would trip it a Send early, so this proves single-increment
/// accounting.
#[tokio::test]
async fn coordinator_service_credential_giveups_count_once_toward_breaker() {
    let cancel = CancellationToken::new();
    let on_disk = r#"{"tokens":{"refresh_token":"same"}}"#;
    let (_tmp, config, _auth) = codex_config_with_auth(on_disk);
    let agent_state = Arc::new(AgentState::new());
    let detector = CredentialFailureDetector::default();

    // Unchanged credential (always gives up) and re-flagged each Turn.
    let backend = Arc::new(CredBackend {
        base: on_disk.to_string(),
        rotate: false,
        counter: AtomicU64::new(0),
        flag: Some(detector.clone()),
    });

    // Three give-up Turns; the fourth Send trips the breaker before submitting.
    let scripts = vec![failing_turn(), failing_turn(), failing_turn()];
    let (tx, received, events) = service_with_credentials(
        HarnessRuntime::scripted_sequence(scripts),
        config,
        Arc::clone(&agent_state),
        backend,
        detector.clone(),
        Arc::new(Dispatcher::new()),
        cancel.clone(),
    );

    // Drive three give-ups, sequencing each Send after the prior reconnect error
    // so the breaker counter is observed one increment at a time.
    for n in 1..=3 {
        send_text(&tx, "do the thing");
        wait_until(
            &events,
            || count_reconnect_errors(&received) >= n,
            "reconnect error",
        )
        .await;
    }
    assert_eq!(
        count_reconnect_errors(&received),
        3,
        "three give-ups each publish one reconnect error"
    );

    // The fourth Send trips the breaker (3 >= MAX_CONSECUTIVE_ERRORS) — proving
    // each give-up incremented the counter exactly once.
    send_text(&tx, "one more");
    wait_until(
        &events,
        || {
            received
                .lock()
                .expect("lock")
                .iter()
                .any(|r| r.payload_json.contains("consecutive"))
        },
        "circuit breaker trips on the fourth send",
    )
    .await;

    // Still exactly three reconnect errors — the breaker short-circuited the
    // fourth Send before it could execute another Turn.
    assert_eq!(
        count_reconnect_errors(&received),
        3,
        "breaker fires on the fourth send, not a fourth give-up"
    );

    cancel.cancel();
}
