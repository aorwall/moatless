//! How a script run that cannot start is reported: a refusal the client can act
//! on, or a defect. Plus what a Task's Scripts resolve to when its Sandbox
//! cannot be reached.

use super::fixtures::*;
use super::logs::CapturedLogs;
use super::*;
use crate::authentication::viewer_scope;
use crate::scripts::resolve;
use crate::startup_scripts::StartupScripts;
use moatless_workspace_types::schemas::repository::ScriptConfig;
use moatless_workspaces::workspace::scripts::ScriptOrigin;

/// A `ScriptsState` over `tasks`, with every other seam unreachable: the
/// cases below stop at the resolve.
fn scripts_state(tasks: ports::MockWorkspaceScriptSource) -> ScriptsState {
    ScriptsState {
        workspace_scripts: Arc::new(tasks),
        drivers: Arc::new(super::fixtures::DrivesNothing),
        ports: unreachable_ports(),
        preview: PreviewState {
            tabs: Arc::new(preview_tabs::PreviewTabs::new(preview_tabs::PreviewTabTtl(
                std::time::Duration::from_hours(1),
            ))),
            tasks: Arc::new(MockTaskReader::new()),
            relay: preview_relay::PreviewRelay::disabled(),
        },
    }
}

/// A stopped Sandbox is a refusal the client answers itself by restarting and
/// rerunning — so it must be a `Fail` with `SandboxNotRunningError`, never a
/// `Die`, or auto-start would need string matching.
#[test]
fn a_stopped_sandbox_refuses_a_script_run_in_terms_the_client_can_act_on() {
    let identity = viewer();
    let error = ScriptsError::Port(SandboxError::NotReady("stopped".to_string()));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, "task-1", &identity);

    let cause = &exit["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a Die would not decode client-side");
    assert_eq!(cause["error"]["_tag"], "SandboxNotRunningError");
    assert_eq!(cause["error"]["threadId"], "task-1");
}

/// A reserved port is the person's configuration to fix, not the client's — it
/// stays a plain refusal, so the typed arm above must not swallow every port
/// failure.
#[test]
fn a_reserved_port_is_still_reported_rather_than_retried() {
    let identity = viewer();
    let error = ScriptsError::Port(SandboxError::BadRequest("Port 8001 is reserved".into()));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, "task-1", &identity);

    assert_eq!(exit["exit"]["cause"][0]["_tag"], "Die");
}

/// A Task Script's record lives in its Sandbox, so a stopped one costs those
/// and not the Workspace's: `moat tasks scripts list` still answers, and
/// `own_unread` is what stops a caller reporting a name it could not look up as
/// a name that is absent.
#[tokio::test]
async fn a_stopped_sandbox_leaves_the_workspace_scripts_listed_and_says_so() {
    let mut tasks = ports::MockWorkspaceScriptSource::new();
    tasks.expect_scripts_for_task().returning(|_, _| {
        Ok(Some(vec![ScriptConfig {
            name: "dev".to_string(),
            label: None,
            command: "bun dev".to_string(),
            icon: None,
            port: None,
            env_vars: None,
            run_on_start: false,
        }]))
    });
    let state = scripts_state(tasks);

    let resolved = resolve(
        &state,
        unreachable_sandboxes().as_ref(),
        &viewer_scope(&viewer()),
        "task-1",
    )
    .await
    .expect("the workspace read answers");

    assert_eq!(resolved.scripts.len(), 1);
    assert_eq!(resolved.scripts[0].config.name, "dev");
    assert_eq!(resolved.scripts[0].declared_by, ScriptOrigin::Workspace);
    assert!(matches!(
        resolved.own_unread,
        Some(SandboxError::NotReady(_))
    ));
}

/// The UI listing runs over the same resolve, so a stopped Sandbox leaves the
/// header's Run control offering the Workspace's Scripts rather than nothing.
#[tokio::test]
async fn listing_a_threads_scripts_answers_over_an_unreachable_sandbox() {
    let mut tasks = ports::MockWorkspaceScriptSource::new();
    tasks.expect_scripts_for_task().returning(|_, _| {
        Ok(Some(vec![ScriptConfig {
            name: "dev".to_string(),
            label: Some("Dev server".to_string()),
            command: "bun dev".to_string(),
            icon: None,
            port: Some(3000),
            env_vars: None,
            run_on_start: false,
        }]))
    });
    let mut state = state();
    state.scripts = scripts_state(tasks);

    let frame = request(
        "1",
        "scripts.listForThread",
        json!({ "threadId": "task-1" }),
    );
    let mut connection = connection();
    let output = connection.exchange(&frame, &state, &viewer()).await;

    let scripts = &output.frames[0]["exit"]["value"]["scripts"];
    // Exactly the Workspace's one: an unreachable Sandbox must drop the Task's
    // own Scripts, not leak a half-read entry through beside them.
    assert_eq!(scripts.as_array().map(Vec::len), Some(1));
    assert_eq!(scripts[0]["id"], "dev");
    assert_eq!(scripts[0]["name"], "Dev server");
    assert_eq!(scripts[0]["scope"], "workspace");
    assert_eq!(scripts[0]["autoOpenPreview"], true);
}

/// A Task the viewer cannot read and one that does not exist are one answer,
/// as everywhere else a scoped read is made — and that answer is an empty
/// listing, not a failure, because the Run control sits on a page that is
/// otherwise working.
#[tokio::test]
async fn listing_the_scripts_of_a_thread_the_viewer_cannot_read_is_empty() {
    let mut tasks = ports::MockWorkspaceScriptSource::new();
    tasks.expect_scripts_for_task().returning(|_, _| Ok(None));
    let mut state = state();
    state.scripts = scripts_state(tasks);

    let frame = request(
        "1",
        "scripts.listForThread",
        json!({ "threadId": "task-1" }),
    );
    let mut connection = connection();
    let output = connection.exchange(&frame, &state, &viewer()).await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(output.frames[0]["exit"]["value"]["scripts"], json!([]));
}

/// A `run_on_start` Task Script whose record could not be read is the failure
/// mode a startup run is for: the Script silently stays down until the next
/// Ready, so the unread sandbox must reach the log instead of resolving to the
/// Workspace's Scripts alone.
#[tokio::test]
async fn a_startup_run_says_when_a_tasks_own_scripts_were_not_read() {
    let mut tasks = ports::MockWorkspaceScriptSource::new();
    tasks.expect_scripts_for_task().returning(|_, _| {
        Ok(Some(vec![ScriptConfig {
            name: "dev".to_string(),
            label: None,
            command: "bun dev".to_string(),
            icon: None,
            port: None,
            env_vars: None,
            run_on_start: true,
        }]))
    });
    let startup = StartupScripts::new(
        scripts_state(tasks),
        state().reads,
        unreachable_sandboxes(),
        terminal::TerminalHub::default(),
    );

    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_max_level(tracing::Level::WARN)
        .finish();
    let failures = {
        let _guard = tracing::subscriber::set_default(subscriber);
        startup
            .run_for_task(&viewer_scope(&viewer()), "task-1")
            .await
            .expect("the workspace read answers")
    };

    let logged = logs.text();
    assert!(
        logged.contains("own run_on_start scripts were not read"),
        "the unread sandbox reached no log: {logged}"
    );
    // The Workspace's own flagged Script was still attempted, and failed for the
    // same stopped Sandbox — that one is reported to the person.
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].name, "dev");
}

/// The Workspace read is the authorization gate, so a Task the viewer cannot
/// read is `NotFound` before the Sandbox is touched at all.
#[tokio::test]
async fn a_task_the_viewer_cannot_read_resolves_to_not_found() {
    let mut tasks = ports::MockWorkspaceScriptSource::new();
    tasks.expect_scripts_for_task().returning(|_, _| Ok(None));
    let state = scripts_state(tasks);

    let refusal = resolve(
        &state,
        unreachable_sandboxes().as_ref(),
        &viewer_scope(&viewer()),
        "task-1",
    )
    .await;

    assert!(matches!(refusal, Err(ScriptsError::NotFound)));
}

/// A portless Script reaches a stopped Sandbox through the terminal, so it must
/// answer with the port path's refusal: `SandboxNotRunningError`, not
/// `TerminalNotRunningError`, or decode fails.
#[test]
fn a_stopped_sandbox_refuses_a_portless_script_run_in_the_same_terms() {
    let identity = viewer();
    let error = ScriptsError::Terminal(terminal::TerminalError::Workspace(
        WorkspaceAccessError::NotRunning(SandboxError::NotReady("stopped".to_string())),
    ));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, "task-1", &identity);

    let cause = &exit["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a Die would not auto-start");
    assert_eq!(cause["error"]["_tag"], "SandboxNotRunningError");
    assert_eq!(cause["error"]["threadId"], "task-1");
}

/// Every other terminal failure is outside this method's declared union, so it
/// goes out as a `Die` the client reports rather than a `Fail` it cannot
/// decode.
#[test]
fn a_terminal_failure_a_script_run_cannot_declare_is_reported_as_a_defect() {
    let identity = viewer();
    let error = ScriptsError::Terminal(terminal::TerminalError::Workspace(
        WorkspaceAccessError::NoWorktree,
    ));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, "task-1", &identity);

    assert_eq!(exit["exit"]["cause"][0]["_tag"], "Die");
}
