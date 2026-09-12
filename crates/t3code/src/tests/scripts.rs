//! How a script run that cannot start is reported: a refusal the client can act
//! on, or a defect.

use super::fixtures::*;
use super::*;

/// A stopped Sandbox is the one `scripts.run` refusal the client answers by
/// itself — it starts the Sandbox and runs the Script again — so it goes out
/// as a `Fail` carrying `SandboxNotRunningError`, not a `Die` carrying a
/// sentence. The tag and the `threadId` are the contract
/// (`packages/contracts/src/sandbox.ts`); a `Die` here would put auto-start
/// behind string matching.
#[test]
fn a_stopped_sandbox_refuses_a_script_run_in_terms_the_client_can_act_on() {
    let identity = viewer();
    let payload = json!({ "threadId": "task-1", "scriptId": "dev" });
    let error = ScriptsError::Port(SandboxError::NotReady("stopped".to_string()));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, &payload, &identity);

    let cause = &exit["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a Die would not decode client-side");
    assert_eq!(cause["error"]["_tag"], "SandboxNotRunningError");
    assert_eq!(cause["error"]["threadId"], "task-1");
}

/// A reserved port is the person's configuration to fix, not the client's —
/// it stays a plain refusal, so the typed arm above must not swallow every
/// port failure.
#[test]
fn a_reserved_port_is_still_reported_rather_than_retried() {
    let identity = viewer();
    let payload = json!({ "threadId": "task-1", "scriptId": "dev" });
    let error = ScriptsError::Port(SandboxError::BadRequest("Port 8001 is reserved".into()));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, &payload, &identity);

    assert_eq!(exit["exit"]["cause"][0]["_tag"], "Die");
}

/// A Script that publishes no port reaches a stopped Sandbox through the
/// terminal it would have run in, and that path has to answer with the same
/// typed refusal the port path does. `scripts.run` declares
/// `SandboxNotRunningError` and not the `Terminal*` tags, so a
/// `TerminalNotRunningError` here fails the client's own decode and the run ends
/// in a schema error instead of an auto-start.
#[test]
fn a_stopped_sandbox_refuses_a_portless_script_run_in_the_same_terms() {
    let identity = viewer();
    let payload = json!({ "threadId": "task-1", "scriptId": "dev" });
    let error = ScriptsError::Terminal(terminal::TerminalError::Workspace(
        WorkspaceAccessError::NotRunning(SandboxError::NotReady("stopped".to_string())),
    ));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, &payload, &identity);

    let cause = &exit["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a Die would not auto-start");
    assert_eq!(cause["error"]["_tag"], "SandboxNotRunningError");
    assert_eq!(cause["error"]["threadId"], "task-1");
}

/// Every other terminal failure is outside this method's declared union, so it
/// goes out as a `Die` the client reports rather than a `Fail` it cannot decode.
#[test]
fn a_terminal_failure_a_script_run_cannot_declare_is_reported_as_a_defect() {
    let identity = viewer();
    let payload = json!({ "threadId": "task-1", "scriptId": "dev" });
    let error = ScriptsError::Terminal(terminal::TerminalError::Workspace(
        WorkspaceAccessError::NoWorktree,
    ));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, &payload, &identity);

    assert_eq!(exit["exit"]["cause"][0]["_tag"], "Die");
}
