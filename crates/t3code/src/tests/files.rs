//! Reading a file under a cwd, where an unreachable cwd is indistinguishable
//! from an unknown one.

use super::fixtures::*;
use super::*;

/// Each method answers in its contract's shape, not as a defect. A `Die`
/// strips the reason from the panel message, so an actionable refusal must
/// arrive as a `Fail` carrying the sentence.
#[tokio::test]
async fn a_refused_file_request_fails_with_the_method_s_own_error() {
    for (tag, payload, error_tag) in [
        (
            "projects.listEntries",
            json!({ "cwd": "/moatless/workspaces/ws-a" }),
            "ProjectListEntriesError",
        ),
        (
            "projects.readFile",
            json!({ "cwd": "/moatless/workspaces/ws-a", "relativePath": "README.md" }),
            "ProjectReadFileError",
        ),
        (
            "projects.goToDefinition",
            json!({ "cwd": "/moatless/workspaces/ws-a", "relativePath": "src/a.ts", "line": 3, "column": 7 }),
            "ProjectGoToDefinitionError",
        ),
    ] {
        let mut connection = connection();
        let output = connection
            .exchange(&request("7", tag, payload), &state(), &viewer())
            .await;
        let exit = &output.frames[0]["exit"];
        assert_eq!(exit["_tag"], "Failure", "{tag}");
        assert_eq!(exit["cause"][0]["_tag"], "Fail", "{tag}");
        assert_eq!(exit["cause"][0]["error"]["_tag"], error_tag);
        assert_eq!(
            exit["cause"][0]["error"]["message"],
            "This conversation has no sandbox, so it has no workspace yet.",
            "a project root is the cwd a thread with no worktree sends"
        );
    }
}

/// A `cwd` naming a conversation the viewer's listing does not contain is
/// answered the same way as one naming nothing at all. The stub listing is
/// empty, so this is every conversation.
#[tokio::test]
async fn a_cwd_outside_the_viewer_s_listing_is_indistinguishable_from_an_unknown_one() {
    for cwd in [
        "/moatless/tasks/task-a",
        "/somewhere/else",
        "/moatless/tasks/",
    ] {
        let mut connection = connection();
        let output = connection
            .exchange(
                &request("8", "projects.listEntries", json!({ "cwd": cwd })),
                &state(),
                &viewer(),
            )
            .await;
        let exit = &output.frames[0]["exit"];
        assert_eq!(exit["cause"][0]["_tag"], "Fail", "{cwd}");
        assert_eq!(
            exit["cause"][0]["error"]["message"], "This conversation's workspace is not available.",
            "{cwd} must not report whether the conversation exists"
        );
    }
}
