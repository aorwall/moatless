//! Serving an environment's own files through the contract's asset method.

use super::fixtures::*;
use super::*;

/// The client decodes a method's failure against that method's declared error
/// type: a tagged error is one of its ordinary outcomes, a string defect is
/// not.
#[tokio::test]
async fn an_asset_this_environment_has_no_file_for_fails_in_the_contracts_own_type() {
    let mut session = session();
    let resource = json!({ "_tag": "native-app-icon" });
    let output = session
        .handle_json(
            &request("8", "assets.createUrl", json!({ "resource": resource })),
            &state(),
            &viewer(),
        )
        .await;
    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail");
    assert_eq!(cause["error"]["_tag"], "AssetWorkspaceAssetNotFoundError");
    assert_eq!(
        cause["error"]["resource"], resource,
        "the error carries back the resource the request named"
    );
}

/// The client keeps "no icon" only when told so as a success; a refusal it
/// asks again for on every reconnect.
#[tokio::test]
async fn a_project_favicon_is_answered_as_missing_rather_than_refused() {
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
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    let url = output.frames[0]["exit"]["value"]["relativeUrl"]
        .as_str()
        .expect("a URL");
    assert!(url.ends_with("/project-favicon-missing"), "{url}");
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
