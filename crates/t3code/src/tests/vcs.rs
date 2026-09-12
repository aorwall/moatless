//! Branches, refs and working-tree status for a thread's checkout.

use super::fixtures::*;
use super::*;

/// The client polls this for every conversation it holds, most of which
/// have no Sandbox running to ask.
#[tokio::test]
async fn list_refs_answers_a_conversation_with_no_sandbox_rather_than_failing() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request(
                "98",
                "vcs.listRefs",
                json!({
                    "cwd": "/moatless/tasks/task-a",
                    "query": "main",
                    "limit": 10
                }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(
        output.frames[0]["exit"]["value"],
        json!({
            "refs": [],
            "isRepo": false,
            "hasPrimaryRemote": false,
            "nextCursor": null,
            "totalCount": 0
        })
    );
}

/// Picking a branch for a draft thread is answered as done, echoing the
/// ref, so the client records it and sends it when the Task is created —
/// there is no checkout to switch yet.
#[tokio::test]
async fn switching_a_draft_threads_branch_echoes_the_ref() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "97",
                "vcs.switchRef",
                json!({ "cwd": "/moatless/workspaces/ws-a", "refName": "feature" }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(
        output.frames[0]["exit"]["value"],
        json!({ "refName": "feature" })
    );
}

/// A thread that already has a Sandbox cannot change branch — that is fixed
/// at creation — so a switch addressed to a worktree is refused rather than
/// silently claimed as done.
#[tokio::test]
async fn switching_an_existing_threads_branch_is_unsupported() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "96",
                "vcs.switchRef",
                json!({ "cwd": "/moatless/tasks/task-a", "refName": "feature" }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["error"]["_tag"], "UnsupportedMethodError");
}

/// The diff panel is a surface the person opened on purpose, so a
/// conversation it cannot read says why — in the error type the method
/// declares, which is what the panel renders instead of a defect.
#[tokio::test]
async fn a_refused_diff_preview_fails_with_the_method_s_own_error() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request(
                "99",
                "review.getDiffPreview",
                json!({ "cwd": "/moatless/tasks/task-a", "ignoreWhitespace": true }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    let cause = &output.frames[0]["exit"]["cause"][0]["error"];
    assert_eq!(cause["_tag"], "GitCommandError");
    assert_eq!(cause["operation"], "review.getDiffPreview");
    assert_eq!(
        cause["detail"], "This conversation's workspace is not available.",
        "a cwd the viewer's listing does not hold must not report whether the conversation exists"
    );
}

/// Opened for every conversation the client shows, including the ones this
/// surface has no path to. A stream that says nothing is the answer there;
/// a failure would be one the client keeps.
#[tokio::test]
async fn a_vcs_status_subscription_without_an_addressable_cwd_simply_stays_quiet() {
    let mut session = session();
    let output = session
        .handle_json(
            &request("100", "subscribeVcsStatus", json!({ "cwd": "   " })),
            &state(),
            &viewer(),
        )
        .await;

    assert!(output.frames.is_empty(), "no chunk, and no failure either");
}

/// The client calls this one on its own schedule and when a worktree is
/// cleaned up, so an unanswerable `cwd` has to come back as the method's
/// error rather than the defect an unimplemented method produces.
#[tokio::test]
async fn a_status_refresh_answers_rather_than_dying() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request(
                "101",
                "vcs.refreshStatus",
                json!({ "cwd": "/moatless/tasks/task-a" }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a typed refusal, not a Die defect");
    assert_eq!(cause["error"]["_tag"], "GitCommandError");
    assert_eq!(cause["error"]["operation"], "vcs.refreshStatus");
}
