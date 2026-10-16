//! A refusal the client can act on, told apart from a defect it cannot.

use super::fixtures::*;
use super::*;

/// The client reports focus changes so a host can idle work. Moatless runs
/// agents in Sandboxes no tab is attached to, so the report is accepted and
/// dropped — refusing it would break tab switching.
#[tokio::test]
async fn a_client_activity_report_is_accepted() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "9",
                "server.reportClientActivity",
                json!({ "state": "active" }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(output.frames[0]["exit"]["value"], Value::Null);
}

/// A `Fail` renders its own message; a `Die` renders only "unexpected server
/// error". A method this server does not have is a typed absence, not a defect,
/// so it arrives as a `Fail`.
#[tokio::test]
async fn a_method_this_server_does_not_have_says_so_in_a_type_the_client_reads() {
    let mut session = session();
    let output = session
        .handle_json(
            &request("1", "orchestration.getFullThreadDiff", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a typed absence, not a Die defect");
    assert_eq!(cause["error"]["_tag"], "UnsupportedMethodError");
    // Carried separately from the message so a client fanning out several
    // calls can tell which one came back absent without parsing text.
    assert_eq!(cause["error"]["method"], "orchestration.getFullThreadDiff");
}

/// Found in production by `moatless_ui_rpc_requests_total{outcome="die"}`:
/// opening a thread the viewer cannot read answered `Die`, so the panel could
/// say only that the server was broken.
#[test]
fn a_thread_the_viewer_cannot_read_is_a_typed_refusal_not_a_defect() {
    let frame = sandbox_failure_exit(
        &request_id("1"),
        Scope::OrchestrationRead,
        "sandbox.subscribeStatus",
        &SandboxRpcError::NotFound,
        &viewer(),
    );

    let cause = &frame["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a refusal, not a Die defect");
    assert_eq!(cause["error"]["_tag"], "EnvironmentAuthorizationError");
    assert_eq!(cause["error"]["requiredScope"], "orchestration:read");
    assert_eq!(cause["error"]["message"], THREAD_UNAVAILABLE);
}

/// The narrowing only moves `NotFound`. A read that actually broke is still a
/// defect, because no client action answers it.
#[test]
fn a_sandbox_that_could_not_be_read_is_still_a_defect() {
    let frame = sandbox_failure_exit(
        &request_id("1"),
        Scope::OrchestrationRead,
        "sandbox.status",
        &SandboxRpcError::Database(sqlx::Error::RowNotFound),
        &viewer(),
    );

    assert_eq!(frame["exit"]["cause"][0]["_tag"], "Die");
}

/// The seam that found this now reads it as a refusal, so it stops counting
/// against the defect rate and starts counting against the refusal one.
#[test]
fn the_metrics_seam_reads_an_unreadable_thread_as_unauthorized() {
    let frames = vec![sandbox_failure_exit(
        &request_id("1"),
        Scope::OrchestrationRead,
        "sandbox.subscribeStatus",
        &SandboxRpcError::NotFound,
        &viewer(),
    )];

    assert_eq!(
        rpc_metrics::classify(&frames),
        rpc_metrics::Outcome::Unauthorized
    );
}

/// Pairing advertises a scope list. Until this gate existed the list was
/// decoration — every method ran whatever it said.
#[tokio::test]
async fn a_method_needing_a_scope_this_deployment_withholds_is_refused() {
    let mut session = session();
    let output = session
        .handle_json(
            &request("1", "cloud.installRelayClient", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail");
    assert_eq!(cause["error"]["_tag"], "EnvironmentAuthorizationError");
    assert_eq!(cause["error"]["requiredScope"], "relay:write");
}

/// `preview.list` declares `EnvironmentAuthorizationError` alone, so a Task
/// this viewer cannot read has to be that rather than a `Die` defect. The
/// client asks it for a draft thread the server has never seen.
#[test]
fn a_preview_of_an_unreadable_thread_is_unauthorized() {
    let frames = crate::rpc::errors::preview_answer(
        &request_id("1"),
        Scope::OrchestrationRead,
        Err(crate::preview::PreviewError::NotFound),
    );

    let cause = &frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail");
    assert_eq!(cause["error"]["_tag"], "EnvironmentAuthorizationError");
    assert_eq!(cause["error"]["message"], THREAD_UNAVAILABLE);
    assert_eq!(
        rpc_metrics::classify(&frames),
        rpc_metrics::Outcome::Unauthorized
    );
}
