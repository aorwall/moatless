//! Serving an environment's own files through the contract's asset method.

use super::fixtures::*;
use super::*;

/// The client decodes a method's failure against that method's declared error
/// type: a tagged error is one of its ordinary outcomes, a string defect is
/// not.
///
/// Answering "no favicon" with a defect leaves the same blank icon behind a
/// stack trace.
#[tokio::test]
async fn an_asset_this_environment_has_no_file_for_fails_in_the_contracts_own_type() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "8",
                "assets.createUrl",
                json!({ "resource": { "_tag": "project-favicon", "cwd": "/moatless" } }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail");
    assert_eq!(cause["error"]["_tag"], "AssetProjectFaviconNotFoundError");
    assert_eq!(
        cause["error"]["resource"],
        json!({ "_tag": "project-favicon", "cwd": "/moatless" }),
        "the error carries back the resource the request named"
    );
}

#[tokio::test]
async fn a_media_file_is_minted_rather_than_refused() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "9",
                "assets.createUrl",
                json!({ "resource": {
                    "_tag": "media-file", "threadId": "t-1",
                    "path": "/moatless/tasks/t-1/docs/shot.png"
                }}),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(
        output.frames[0]["exit"]["value"]["relativeUrl"],
        "/api/assets/t-1/docs/shot.png"
    );
}
