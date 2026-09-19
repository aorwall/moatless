//! The project rows the listing carries, their defaults, and the scripts a
//! viewer may read.

use super::contract::*;
use super::fixtures::*;
use super::*;

/// The client groups by project and drops a conversation whose project it
/// cannot find, in silence. Leaving it out here costs the same row and leaves
/// something to diagnose it by.
#[test]
fn a_conversation_with_no_listable_project_is_left_out() {
    let mut orphan = conversation("task-orphan", "ws-a");
    orphan.workspace_id = None;
    let mut elsewhere = conversation("task-elsewhere", "ws-a");
    elsewhere.workspace_id = Some("ws-not-listed".to_string());

    let listed = threads(
        &[workspace("ws-a", "Alpha")],
        &[conversation("task-a", "ws-a"), orphan, elsewhere],
    );
    assert_eq!(
        listed.iter().map(|t| t["id"].clone()).collect::<Vec<_>>(),
        vec![json!("task-a")]
    );
}

#[test]
fn a_blank_workspace_name_falls_back_to_the_id() {
    let project = project(&workspace("ws-a", "   "));
    assert_eq!(project["title"], "ws-a");
}

#[test]
fn a_script_that_cannot_fill_the_required_fields_is_left_out() {
    let mut ws = workspace("ws-a", "Alpha");
    ws.workspace.scripts = Some(json!([
        { "name": "  ", "command": "bun run dev" },
        { "name": "blank-command", "command": "  " },
        { "name": "no-command" }
    ]));
    assert!(project(&ws)["scripts"].as_array().unwrap().is_empty());
}

#[test]
fn each_workspace_gets_a_distinct_root_so_groups_do_not_collapse() {
    let snapshot = shell_snapshot(
        &[workspace("ws-a", "Alpha"), workspace("ws-b", "Beta")],
        &[],
        &AgentModelDefaults::default(),
        0,
    );
    let projects = snapshot["projects"].as_array().unwrap();

    assert_eq!(projects[0]["id"], "ws-a");
    assert_eq!(projects[0]["title"], "Alpha");
    assert_eq!(projects[1]["id"], "ws-b");
    assert_ne!(projects[0]["workspaceRoot"], projects[1]["workspaceRoot"]);
}

#[test]
fn the_workspace_defaults_reach_the_client_as_an_icon_and_a_model_selection() {
    let mut ws = workspace("ws-a", "Alpha");
    ws.workspace.icon = Some(json!({ "kind": "lucide", "name": "rocket", "color": "blue" }));
    ws.workspace.default_model = Some("opus".to_string());
    ws.workspace.default_effort = Some("max".to_string());

    let project = project(&ws);

    assert_eq!(project["projectIcon"]["name"], "rocket");
    assert_eq!(
        project["defaultModelSelection"],
        json!({
            "instanceId": "claudeAgent",
            "model": "claude-opus-5",
            "options": [{ "id": "effort", "value": "max" }],
        }),
        "the catalog id is published as T3's own slug, under the instance's effort descriptor"
    );
}

#[test]
fn a_model_this_catalog_does_not_publish_selects_nothing() {
    let mut ws = workspace("ws-a", "Alpha");
    ws.workspace.default_model = Some("retired-model".to_string());
    assert_eq!(project(&ws)["defaultModelSelection"], Value::Null);

    // The `default` sentinel names no model, so it is not a selection either.
    ws.workspace.default_model = Some(catalog::UNRESOLVED_MODEL_CHOICE.to_string());
    assert_eq!(project(&ws)["defaultModelSelection"], Value::Null);
}

#[test]
fn the_widened_half_of_the_listing_discloses_neither_default() {
    let mut ws = a_referenced_workspace("ws-a", "Alpha");
    ws.workspace.icon = Some(json!({ "kind": "emoji", "emoji": "🚀" }));
    ws.workspace.default_model = Some("opus".to_string());

    let project = project(&ws);

    assert_eq!(project["projectIcon"], Value::Null);
    assert_eq!(project["defaultModelSelection"], Value::Null);
}

#[test]
fn an_effort_the_selected_model_does_not_offer_selects_nothing() {
    let mut ws = workspace("ws-a", "Alpha");
    ws.workspace.default_model = Some("gpt-5.6-luna".to_string());
    ws.workspace.default_effort = Some("ultra".to_string());

    // `ultra` is a level its sibling models take and this one rejects, and
    // Codex fails the turn on one it does not know.
    assert_eq!(
        project(&ws)["defaultModelSelection"]["options"],
        Value::Null
    );

    ws.workspace.default_model = Some("gpt-5.6-sol".to_string());
    assert_eq!(
        project(&ws)["defaultModelSelection"]["options"],
        json!([{ "id": "reasoningEffort", "value": "ultra" }])
    );

    // Whitespace is not a level either.
    ws.workspace.default_effort = Some("  ".to_string());
    assert_eq!(
        project(&ws)["defaultModelSelection"]["options"],
        Value::Null
    );
}

#[test]
fn an_icon_the_clients_schema_rejects_is_not_published() {
    let mut ws = workspace("ws-a", "Alpha");
    for icon in [
        json!({ "kind": "lucide", "name": "rocket", "color": "puce" }),
        json!({ "kind": "lucide", "name": "Rocket Ship", "color": "blue" }),
        json!({ "kind": "lucide", "name": "rocket" }),
        json!({ "kind": "emoji", "emoji": "" }),
        json!({ "kind": "sticker", "sticker": "rocket" }),
        json!("rocket"),
    ] {
        ws.workspace.icon = Some(icon.clone());
        assert_eq!(
            project(&ws)["projectIcon"],
            Value::Null,
            "the client drops a project whose icon it cannot decode: {icon}"
        );
    }

    ws.workspace.icon = Some(json!({ "kind": "emoji", "emoji": " 🚀 " }));
    assert_eq!(
        project(&ws)["projectIcon"],
        json!({ "kind": "emoji", "emoji": "🚀" }),
        "the schema takes a trimmed string, so the stored one is trimmed"
    );
}

#[test]
fn declared_scripts_become_scripts_and_servers_do_not() {
    let mut ws = workspace("ws-a", "Alpha");
    ws.workspace.setup_commands = Some(json!(["bun install"]));
    // Servers must contribute no script at all: pressing one is a bug.
    ws.workspace.servers = Some(json!([
        { "name": "dev", "port": 3000, "startCommands": ["bun run dev"], "default": true }
    ]));
    ws.workspace.scripts = Some(json!([
        {
            "name": "test",
            "label": "Run tests",
            "command": "bun test",
            "icon": "test"
        },
        {
            "name": "storybook",
            "command": "bun run storybook",
            "port": 6006
        },
        { "name": "no-command" }
    ]));

    let scripts = project(&ws)["scripts"].as_array().unwrap().clone();

    assert_eq!(
        scripts.len(),
        2,
        "a script with no command is left out, and servers contribute none"
    );
    assert_eq!(scripts[0]["id"], "test");
    assert_eq!(scripts[0]["name"], "Run tests");
    assert_eq!(scripts[0]["command"], "bun test");
    assert_eq!(scripts[0]["icon"], "test");
    assert_eq!(
        scripts[0]["autoOpenPreview"], false,
        "a script with no port does not auto-open a preview"
    );
    assert_eq!(scripts[1]["id"], "storybook");
    assert_eq!(scripts[1]["name"], "storybook", "name falls back to the id");
    assert_eq!(
        scripts[1]["autoOpenPreview"], true,
        "a script with a port auto-opens its preview"
    );
    assert!(
        scripts[1].get("previewUrl").is_none(),
        "a bare port is not a URL, so the field is left out"
    );
    assert!(
        scripts[0].get("scope").is_none(),
        "only the merged per-thread listing says where a script came from"
    );
}

#[test]
fn an_unknown_icon_falls_back_rather_than_failing_the_listing() {
    let mut ws = workspace("ws-a", "Alpha");
    ws.workspace.scripts = Some(json!([
        { "name": "weird", "command": "echo hi", "icon": "rocket" }
    ]));
    let scripts = project(&ws)["scripts"].as_array().unwrap().clone();
    assert_eq!(scripts[0]["icon"], "play");
}

/// The widened half of the project union carries what renders a group and
/// nothing else.
///
/// Asserted against the *same row* projected both ways, so the only variable is
/// which half of the union it came from; treating the halves alike fails one of
/// the two `scripts` assertions.
#[test]
fn a_project_the_viewer_may_not_read_carries_no_scripts() {
    let scripts = json!([{
        "name": "dev",
        "command": "bun run dev --config /home/them/secret.toml"
    }]);
    let mut readable = workspace("ws-a", "Alpha");
    readable.workspace.scripts = Some(scripts.clone());
    let mut referenced = a_referenced_workspace("ws-a", "Alpha");
    referenced.workspace.scripts = Some(scripts);

    assert_eq!(
        project(&readable)["scripts"][0]["command"],
        "bun run dev --config /home/them/secret.toml"
    );
    assert_eq!(project(&referenced)["scripts"], json!([]));

    // Still decodable, and still enough to render the group the widening
    // exists for.
    let projected = project(&referenced);
    assert_keys(&projected, "project", PROJECT_REQUIRED, PROJECT_OPTIONAL);
    assert_non_empty(&projected, "project", PROJECT_NON_EMPTY);
    assert_eq!(projected["title"], "Alpha");
}

#[test]
fn an_unreadable_scripts_config_reports_no_scripts() {
    let mut ws = workspace("ws-a", "Alpha");
    ws.workspace.scripts = Some(json!({ "not": "a list" }));
    assert!(project(&ws)["scripts"].as_array().unwrap().is_empty());
}
