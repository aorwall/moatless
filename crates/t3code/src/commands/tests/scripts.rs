//! `project.meta.update`: Scripts are edited in Git, and Moatless owns the
//! other project fields.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn a_scripts_update_is_rejected_toward_the_git_file() {
    let (result, calls) = dispatch_against(
        conversations(None),
        json!({
            "type": "project.meta.update",
            "commandId": "cmd-1",
            "projectId": "ws-1",
            "scripts": [{"id": "dev", "name": "dev", "command": "pnpm dev"}]
        }),
    )
    .await;
    let Err(CommandRefusal::Rejected { message }) = result else {
        panic!("expected rejection: {result:?}")
    };
    assert!(message.contains(".moatless/workspaces.json"), "{message}");
    assert!(calls.is_empty());
}

#[tokio::test]
async fn project_fields_owned_by_moatless_are_refused() {
    for field in ["title", "workspaceRoot", "faviconPath"] {
        let (result, _) = dispatch_against(
            conversations(None),
            json!({
                "type": "project.meta.update",
                "commandId": "cmd-1",
                "projectId": "ws-1",
                "scripts": [],
                field: "something"
            }),
        )
        .await;
        assert!(
            matches!(result, Err(CommandRefusal::Unsupported { .. })),
            "{result:?}"
        );
    }
}

#[tokio::test]
async fn project_update_without_scripts_is_accepted() {
    let (result, _) = dispatch_against(
        conversations(None),
        json!({"type": "project.meta.update", "commandId": "cmd-1", "projectId": "ws-1"}),
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
}
