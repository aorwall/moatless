use super::*;

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_780_000_000 + seconds, 0).expect("valid instant")
}

/// Events numbered from 1, a second apart, in the order given.
fn log(events: &[(&str, Value)]) -> Vec<SandboxLogEvent> {
    events
        .iter()
        .zip(1..)
        .map(|((sub_type, payload), id)| SandboxLogEvent {
            id,
            sub_type: (*sub_type).to_string(),
            payload: payload.clone(),
            at: at(id),
        })
        .collect()
}

fn accepted() -> (&'static str, Value) {
    ("initializing", json!({ "message": "Sandbox initializing" }))
}

fn scheduled() -> (&'static str, Value) {
    (
        "initializing",
        json!({ "message": "Sandbox lifecycle: pod_scheduled", "lifecycleEvent": "pod_scheduled" }),
    )
}

fn step(reason: &str, message: &str) -> (&'static str, Value) {
    (
        "initializing",
        json!({ "reason": reason, "message": message, "eventType": "Normal" }),
    )
}

fn ready() -> (&'static str, Value) {
    (
        "ready",
        json!({ "lifecycleEvent": "pod_ready", "message": "Ready" }),
    )
}

fn statuses(snapshot: &Value) -> Vec<(String, String)> {
    snapshot["stages"]
        .as_array()
        .expect("stages")
        .iter()
        .map(|stage| {
            (
                stage["id"].as_str().unwrap().to_string(),
                stage["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn expect(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(id, status)| ((*id).to_string(), (*status).to_string()))
        .collect()
}

#[test]
fn a_task_whose_sandbox_never_started_has_no_setup() {
    assert_eq!(snapshot("task-1", &[], at(0)), Value::Null);
    let only_stops = log(&[("stopped", json!({ "message": "Sandbox stopped" }))]);
    assert_eq!(snapshot("task-1", &only_stops, at(5)), Value::Null);
}

#[test]
fn a_start_in_flight_is_running_at_the_step_after_the_last_reported() {
    let events = log(&[
        accepted(),
        scheduled(),
        step("CredentialsFetched", "Fetched credentials from backend"),
    ]);
    let setup = snapshot("task-1", &events, at(4));
    assert_eq!(setup["phase"], "running");
    assert_eq!(setup["threadId"], "task-1");
    assert_eq!(setup["sequence"], 3);
    assert_eq!(setup["endedAt"], Value::Null);
    assert_eq!(
        statuses(&setup),
        expect(&[
            ("fetch", "done"),
            ("checkout", "running"),
            ("submodules", "pending"),
            ("agent", "pending"),
            ("setup-script", "pending"),
        ])
    );
    assert_eq!(
        setup["stages"][0]["detail"],
        "Fetched credentials from backend"
    );
    assert_eq!(setup["stages"][1]["startedAt"], instant(at(3)));
}

/// A Workspace that places no repository reports no checkout, and one with no
/// setup commands reports none of them: both are skipped, not left pending.
#[test]
fn a_no_repo_start_skips_the_checkout_and_finishes_when_ready() {
    let events = log(&[
        accepted(),
        scheduled(),
        step("CredentialsFetched", "Fetched credentials from backend"),
        step("PluginsCloned", "Fetched 1 plugin(s)"),
        ready(),
    ]);
    let setup = snapshot("task-1", &events, at(6));
    assert_eq!(setup["phase"], "done");
    assert_eq!(setup["endedAt"], instant(at(5)));
    assert_eq!(
        statuses(&setup),
        expect(&[
            ("fetch", "done"),
            ("checkout", "skipped"),
            ("submodules", "done"),
            ("agent", "done"),
            ("setup-script", "skipped"),
        ])
    );
}

/// Setup commands run after the agent is ready, so the client stops holding
/// the composer once `agent` is done while the card still shows the command.
#[test]
fn setup_commands_keep_the_start_running_after_the_agent_is_ready() {
    let mut events = vec![
        accepted(),
        step("CredentialsFetched", "Fetched credentials from backend"),
        step("RepositoryCloned", "Cloned web"),
        ready(),
        step("SetupRunning", "Running setup command: bun install"),
    ];
    let setup = snapshot("task-1", &log(&events), at(6));
    assert_eq!(setup["phase"], "running");
    assert_eq!(
        statuses(&setup),
        expect(&[
            ("fetch", "done"),
            ("checkout", "done"),
            ("submodules", "skipped"),
            ("agent", "done"),
            ("setup-script", "running"),
        ])
    );
    assert_eq!(
        setup["stages"][4]["detail"],
        "Running setup command: bun install"
    );

    events.push(step("WorkspaceSetupComplete", "Workspace setup complete"));
    let finished = snapshot("task-1", &log(&events), at(7));
    assert_eq!(finished["phase"], "done");
    assert_eq!(finished["endedAt"], instant(at(6)));
    assert_eq!(finished["stages"][4]["status"], "done");
}

#[test]
fn a_failed_setup_command_is_a_warning_on_a_start_that_finished() {
    let events = log(&[
        accepted(),
        step("CredentialsFetched", "Fetched credentials from backend"),
        ready(),
        step("SetupRunning", "Running setup command: make"),
        step("SetupFailed", "Setup command failed: make"),
    ]);
    let setup = snapshot("task-1", &events, at(6));
    assert_eq!(setup["phase"], "done");
    assert_eq!(setup["stages"][4]["status"], "warning");
    assert_eq!(setup["error"], Value::Null);
}

#[test]
fn a_failed_clone_fails_the_checkout_and_the_start() {
    let events = log(&[
        accepted(),
        step("CredentialsFetched", "Fetched credentials from backend"),
        (
            "initializing",
            json!({ "reason": "RepositoryCloneFailed", "message": "Clone failed: denied", "eventType": "Warning" }),
        ),
    ]);
    let setup = snapshot("task-1", &events, at(4));
    assert_eq!(setup["phase"], "failed");
    assert_eq!(setup["error"], "Clone failed: denied");
    assert_eq!(
        statuses(&setup),
        expect(&[
            ("fetch", "done"),
            ("checkout", "failed"),
            ("submodules", "skipped"),
            ("agent", "skipped"),
            ("setup-script", "skipped"),
        ])
    );
}

#[test]
fn a_lifecycle_failure_fails_the_stage_in_flight() {
    let events = log(&[
        accepted(),
        (
            "error",
            json!({ "message": "Sandbox start failed: no capacity" }),
        ),
    ]);
    let setup = snapshot("task-1", &events, at(3));
    assert_eq!(setup["phase"], "failed");
    assert_eq!(setup["stages"][0]["status"], "failed");
    assert_eq!(setup["error"], "Sandbox start failed: no capacity");
}

#[test]
fn a_stop_before_the_sandbox_is_ready_cancels_the_start() {
    let events = log(&[
        accepted(),
        scheduled(),
        (
            "stopped",
            json!({ "message": "Sandbox stopped", "initiator": "user" }),
        ),
    ]);
    let setup = snapshot("task-1", &events, at(4));
    assert_eq!(setup["phase"], "cancelled");
    assert_eq!(setup["endedAt"], instant(at(3)));
    assert_eq!(setup["error"], Value::Null);
}

#[test]
fn a_stop_after_the_sandbox_was_ready_leaves_the_start_done() {
    let events = log(&[
        accepted(),
        ready(),
        ("stopped", json!({ "message": "Sandbox stopped" })),
    ]);
    assert_eq!(snapshot("task-1", &events, at(4))["phase"], "done");
}

#[test]
fn only_the_newest_start_is_reported() {
    let events = log(&[
        accepted(),
        ("error", json!({ "message": "Sandbox start failed" })),
        accepted(),
        step("CredentialsFetched", "Fetched credentials from backend"),
    ]);
    let setup = snapshot("task-1", &events, at(5));
    assert_eq!(setup["phase"], "running");
    assert_eq!(setup["startedAt"], instant(at(3)));
    assert_eq!(setup["sequence"], 4);
}

#[test]
fn a_start_that_finished_long_ago_is_not_reported_and_one_still_running_is() {
    let finished = log(&[accepted(), ready()]);
    let later = at(2) + FINISHED_START_SHOWN_FOR + Duration::seconds(1);
    assert_eq!(snapshot("task-1", &finished, later), Value::Null);

    let stuck = log(&[accepted(), scheduled()]);
    assert_eq!(snapshot("task-1", &stuck, later)["phase"], "running");
}

#[test]
fn only_a_running_start_can_be_cancelled() {
    let running = snapshot("task-1", &log(&[accepted()]), at(2));
    let done = snapshot("task-1", &log(&[accepted(), ready()]), at(3));
    assert!(is_running(&running));
    assert!(!is_running(&done));
    assert!(!is_running(&Value::Null));
}
