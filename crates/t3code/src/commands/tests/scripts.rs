//! Editing a project's Scripts, and the project fields Moatless owns.

use super::fixtures::*;
use super::*;

/// A `project.meta.update` carrying `scripts`, as the client's scripts menu
/// sends it — the whole list as it should now be.
fn scripts_update(scripts: Value) -> Value {
    json!({
        "type": "project.meta.update",
        "commandId": "cmd-1",
        "projectId": "ws-1",
        "scripts": scripts
    })
}

fn wire_script(id: &str, command: &str) -> Value {
    json!({"id": id, "name": id, "command": command, "icon": "play", "runOnWorktreeCreate": false})
}

#[tokio::test]
async fn script_edits_decode_wire_fields_and_delegate_to_workspace_owner() {
    let mut commands = MockWorkspaceScriptCommands::new();
    commands
        .expect_replace_scripts()
        .withf(|_, workspace_id, scripts| {
            workspace_id == "ws-1"
                && scripts.len() == 1
                && scripts[0].name == "dev"
                && scripts[0].command == "pnpm dev"
                && scripts[0].port == Some(3000)
                && scripts[0].env_vars.is_none()
                && scripts[0].run_on_start
        })
        .once()
        .returning(|_, _, _| Ok(()));
    let mut script = wire_script("dev", "pnpm dev");
    script["port"] = json!(3000);
    script["runOnWorktreeCreate"] = json!(true);
    let (result, calls) = dispatch_with_workspaces(
        conversations(None),
        commands,
        scripts_update(json!([script])),
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
    assert!(calls.is_empty());
}

#[tokio::test]
async fn workspace_write_refusals_keep_the_protocol_mapping() {
    for (error, message_part) in [
        (ScriptsWriteError::GitSynced, "workspaces.json"),
        (ScriptsWriteError::NotFound, "not one you can change"),
        (
            ScriptsWriteError::Invalid("duplicate name".into()),
            "duplicate name",
        ),
    ] {
        let mut commands = MockWorkspaceScriptCommands::new();
        commands
            .expect_replace_scripts()
            .once()
            .return_once(|_, _, _| Err(error));
        let (result, _) = dispatch_with_workspaces(
            conversations(None),
            commands,
            scripts_update(json!([wire_script("dev", "pnpm dev")])),
        )
        .await;
        let Err(CommandRefusal::Rejected { message }) = result else {
            panic!("expected rejection: {result:?}")
        };
        assert!(message.contains(message_part), "{message}");
    }
}

#[tokio::test]
async fn invalid_wire_script_never_reaches_workspace_owner() {
    let mut commands = MockWorkspaceScriptCommands::new();
    commands.expect_replace_scripts().never();
    let mut script = wire_script("dev", "pnpm dev");
    script["port"] = json!(70000);
    let (result, _) = dispatch_with_workspaces(
        conversations(None),
        commands,
        scripts_update(json!([script])),
    )
    .await;
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
}

#[tokio::test]
async fn project_fields_owned_by_moatless_are_refused() {
    for field in ["title", "workspaceRoot", "faviconPath"] {
        let mut command = scripts_update(json!([]));
        command[field] = json!("something");
        let (result, _) = dispatch_with_workspaces(
            conversations(None),
            MockWorkspaceScriptCommands::new(),
            command,
        )
        .await;
        assert!(
            matches!(result, Err(CommandRefusal::Unsupported { .. })),
            "{result:?}"
        );
    }
}

#[tokio::test]
async fn project_update_without_scripts_does_not_write() {
    let mut commands = MockWorkspaceScriptCommands::new();
    commands.expect_replace_scripts().never();
    let (result, _) = dispatch_with_workspaces(
        conversations(None),
        commands,
        json!({"type": "project.meta.update", "commandId": "cmd-1", "projectId": "ws-1"}),
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
}
