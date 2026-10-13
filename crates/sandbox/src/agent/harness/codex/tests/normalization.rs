//! Native normalization: one test per item family.

use super::fixtures::fixture_item;
use super::fixtures::*;
use super::*;

/// The whole mapping table in one place: every family the wire can deliver,
/// against the canonical blocks it produces.
///
/// A family that renders nothing produces none deliberately, distinct from an
/// unknown family, which is forwarded raw (see
/// `unknown_item_family_is_forwarded_raw`).
#[test]
fn every_item_family_maps_to_its_canonical_blocks() {
    let expected: &[(&str, &[(&str, &str)])] = &[
        // Rendered as assistant text.
        ("agentMessage", &[("text", "-")]),
        ("reasoning", &[("text", "-")]),
        // Tool families: a tool_use plus its paired tool_result.
        (
            "commandExecution",
            &[("tool_use", "codex.shell"), ("tool_result", "-")],
        ),
        (
            "fileChange",
            &[("tool_use", "codex.edit"), ("tool_result", "-")],
        ),
        (
            "mcpToolCall",
            &[("tool_use", "codex.mcp_tool_call"), ("tool_result", "-")],
        ),
        // A namespaced tool with JSON arguments and content items back is what
        // `codex.mcp_tool_call` already renders.
        (
            "dynamicToolCall",
            &[("tool_use", "codex.mcp_tool_call"), ("tool_result", "-")],
        ),
        (
            "collabAgentToolCall",
            &[("tool_use", "codex.collab_tool_call"), ("tool_result", "-")],
        ),
        // Web search has no result concept, so it is a lone tool_use.
        ("webSearch", &[("tool_use", "codex.web_search")]),
        // The nine families with nothing to render. The prompt echo is already
        // in the conversation; the rest are status notices with no canonical
        // block type to land in.
        ("userMessage", &[]),
        ("hookPrompt", &[]),
        ("plan", &[]),
        ("subAgentActivity", &[]),
        ("imageView", &[]),
        ("sleep", &[]),
        ("imageGeneration", &[]),
        ("enteredReviewMode", &[]),
        ("exitedReviewMode", &[]),
        ("contextCompaction", &[]),
    ];

    for (family, item) in every_family() {
        let want = expected
            .iter()
            .find(|(name, _)| *name == family)
            .unwrap_or_else(|| panic!("{family} has no entry in the mapping table"))
            .1;
        let got = blocks_for(item);
        let got: Vec<(&str, &str)> = got
            .iter()
            .map(|(ty, name)| (ty.as_str(), name.as_str()))
            .collect();
        assert_eq!(got, want, "{family} maps to the wrong blocks");
    }
}

#[test]
fn the_mapping_table_covers_every_family_the_transport_models() {
    // A new family added to `ThreadItem` without a fixture would go unmapped,
    // which the transport's own `every_item_family_has_a_fixture` catches; this
    // pins that the harness reads all eighteen of them.
    assert_eq!(every_family().len(), 18);
}

#[test]
fn agent_message_and_reasoning_are_ordered_assistant_text_blocks() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!("observed" / "item_reasoning")));
    p.absorb(item_completed(fixture_item!(
        "observed" / "item_agent_message"
    )));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].block_json()["type"], "text");
    assert_eq!(
        s[0].block_json()["text"],
        "**Planning update and patch sequence**"
    );
    assert_eq!(s[1].block_json()["type"], "text");
    assert!(
        s[1].block_json()["text"]
            .as_str()
            .expect("text")
            .contains("four steps")
    );
}

/// `codex exec` reported reasoning as one `text`; this transport splits it into
/// `summary` and `content`, and an item with neither is normal.
#[test]
fn reasoning_falls_back_from_summary_to_content_and_may_render_nothing() {
    let summary = blocks_for(json!({
        "type": "reasoning", "id": "rs_0",
        "summary": ["first", "second"], "content": ["raw"],
    }));
    assert_eq!(summary.len(), 1);

    let mut p = open_process();
    p.absorb(item_completed(json!({
        "type": "reasoning", "id": "rs_0",
        "summary": ["first", "second"], "content": ["raw"],
    })));
    let events = drain(&mut p);
    assert_eq!(
        sources(&events)[0].block_json()["text"],
        "first\n\nsecond",
        "the summary wins, and multiple parts join as paragraphs"
    );

    let content_only = {
        let mut p = open_process();
        p.absorb(item_completed(
            json!({ "type": "reasoning", "id": "rs_1", "summary": [], "content": ["raw"] }),
        ));
        let events = drain(&mut p);
        sources(&events)[0].block_json()["text"].clone()
    };
    assert_eq!(content_only, "raw", "content is read when summary is empty");

    assert!(
        blocks_for(json!({ "type": "reasoning", "id": "rs_2", "summary": [], "content": [] }))
            .is_empty(),
        "an empty reasoning item renders nothing rather than an empty block"
    );
}

#[test]
fn command_execution_carries_the_same_shell_payload_exec_produced() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "observed" / "item_command_execution"
    )));
    let events = drain(&mut p);
    let s = sources(&events);

    let use_block = s[0].block_json();
    assert_eq!(use_block["name"], "codex.shell");
    assert_eq!(
        use_block["input"]["command"],
        "/bin/bash -lc 'echo hello-from-codex'"
    );
    assert_eq!(use_block["input"]["cwd"], "/tmp/probe/turn/ws");

    let result_block = s[1].block_json();
    assert_eq!(result_block["type"], "tool_result");
    assert_eq!(result_block["tool_use_id"], use_block["id"]);
    assert_eq!(result_block["is_error"], false);
    assert_eq!(result_block["content"], "hello-from-codex\n");
}

#[test]
fn a_failed_command_is_flagged_as_an_error_result() {
    let mut p = open_process();
    p.absorb(item_completed(json!({
        "type": "commandExecution", "id": "call_0",
        "command": "false", "cwd": "/w", "status": "failed", "exitCode": 1,
        "aggregatedOutput": "boom\n",
    })));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s[1].block_json()["is_error"], true);
    assert_eq!(s[1].block_json()["content"], "boom\n");
}

/// The app-server tags the change kind (`{"type":"update"}`) where exec sent a
/// bare `"update"`; the canonical payload must not change shape because of it.
#[test]
fn file_change_kinds_map_to_the_canonical_flat_kind() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "observed" / "item_file_change_add"
    )));
    p.absorb(item_completed(fixture_item!(
        "observed" / "item_file_change_update"
    )));
    p.absorb(item_completed(json!({
        "type": "fileChange", "id": "call_del", "status": "completed",
        "changes": [{ "path": "/gone.txt", "kind": { "type": "delete" }, "diff": "-x\n" }],
    })));
    let events = drain(&mut p);
    let s = sources(&events);

    let add = s[0].block_json();
    assert_eq!(add["name"], "codex.edit");
    assert_eq!(
        add["input"]["changes"][0]["path"],
        "/tmp/probe/turn/ws/note.txt"
    );
    assert_eq!(add["input"]["changes"][0]["kind"], "add");
    assert_eq!(add["input"]["changes"][0]["diff"], "ALPHA\n");
    assert_eq!(
        s[1].block_json()["content"],
        "add /tmp/probe/turn/ws/note.txt"
    );

    assert_eq!(s[2].block_json()["input"]["changes"][0]["kind"], "update");
    assert_eq!(s[4].block_json()["input"]["changes"][0]["kind"], "delete");
}

#[test]
fn mcp_and_dynamic_tool_calls_share_the_mcp_card() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "schema-derived" / "item_mcp_tool_call"
    )));
    p.absorb(item_completed(fixture_item!(
        "schema-derived" / "item_dynamic_tool_call"
    )));
    let events = drain(&mut p);
    let s = sources(&events);

    assert_eq!(s[0].block_json()["input"]["server"], "atlassian");
    assert_eq!(s[0].block_json()["input"]["tool"], "jira_search");
    assert_eq!(s[0].block_json()["input"]["arguments"]["q"], "foo");
    assert_eq!(s[1].block_json()["content"], "match");

    // The namespace is the server, so a dynamic tool renders as a named card
    // rather than an anonymous one.
    assert_eq!(s[2].block_json()["input"]["server"], "acme");
    assert_eq!(s[2].block_json()["input"]["tool"], "lookup");
    assert_eq!(s[3].block_json()["content"], "result text");
}

#[test]
fn a_failed_mcp_call_carries_its_error_message() {
    let mut p = open_process();
    p.absorb(item_completed(json!({
        "type": "mcpToolCall", "id": "call_0", "server": "db", "tool": "q",
        "arguments": {}, "status": "failed", "error": { "message": "connection refused" },
    })));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s[1].block_json()["is_error"], true);
    assert_eq!(s[1].block_json()["content"], "connection refused");
}

#[test]
fn collab_tool_call_carries_its_agents_states() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "schema-derived" / "item_collab_agent_tool_call"
    )));
    let events = drain(&mut p);
    let s = sources(&events);
    let input = s[0].block_json();
    assert_eq!(input["name"], "codex.collab_tool_call");
    assert_eq!(input["input"]["tool"], "spawn_agent");
    assert_eq!(
        input["input"]["senderThreadId"],
        "019f0000-0000-7000-8000-00000000thre"
    );
    assert_eq!(input["input"]["prompt"], "help with the migration");
    assert_eq!(
        s[1].block_json()["content"],
        "019f0000-0000-7000-8000-0000000000sub: running"
    );
}

/// `resumeAgent` has no canonical equivalent — the schema's collab tool enum
/// does not carry it — so it maps to the explicit `unknown` rather than being
/// silently reported as some other tool.
#[test]
fn an_unmodelled_collab_tool_maps_to_unknown() {
    let mut p = open_process();
    p.absorb(item_completed(json!({
        "type": "collabAgentToolCall", "id": "call_0", "tool": "resumeAgent",
        "senderThreadId": "main", "status": "completed",
    })));
    let events = drain(&mut p);
    assert_eq!(sources(&events)[0].block_json()["input"]["tool"], "unknown");
}

#[test]
fn web_search_is_a_lone_tool_use_carrying_the_query() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "observed" / "item_web_search"
    )));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 1, "web search has no result concept");
    assert_eq!(s[0].block_json()["name"], "codex.web_search");
    assert!(
        s[0].block_json()["input"]["query"]
            .as_str()
            .expect("query")
            .contains("Rust release")
    );
    assert_eq!(s[0].block_json()["input"]["action"]["type"], "search");
}

#[test]
fn unknown_item_family_is_forwarded_raw() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "schema-derived" / "item_unknown_family"
    )));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 1);
    // The raw native block is preserved verbatim inside the canonical `unknown`
    // block so an upstream CLI addition is never silently dropped.
    let block = s[0].block_json();
    assert_eq!(block["type"], "unknown");
    assert_eq!(block["raw"]["type"], "brandNewThing");
    assert_eq!(block["raw"]["foo"], "bar");
}

/// The running plan is a notification here, not the re-emitted `todo_list` item
/// exec sent — but it must still upsert one live card.
#[test]
fn plan_updates_share_a_stable_tool_use_id() {
    let mut p = open_process();
    p.absorb(plan_updated(&[("a", "inProgress"), ("b", "pending")]));
    p.absorb(plan_updated(&[("a", "completed"), ("b", "inProgress")]));
    p.absorb(plan_updated(&[("a", "completed"), ("b", "completed")]));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 3, "every update emits a block");
    assert_eq!(s[0].block_json()["id"], s[1].block_json()["id"]);
    assert_eq!(s[1].block_json()["id"], s[2].block_json()["id"]);
    assert_eq!(s[0].block_json()["name"], "codex.todo_list");
    assert_eq!(s[0].block_json()["input"]["items"][0]["text"], "a");
    assert_eq!(s[0].block_json()["input"]["items"][0]["completed"], false);
    assert_eq!(s[2].block_json()["input"]["items"][0]["completed"], true);
    // Only `completed` counts as done; in-progress is still outstanding.
    assert_eq!(s[1].block_json()["input"]["items"][1]["completed"], false);
}

#[test]
fn duplicate_item_ids_across_spawns_get_distinct_tool_use_ids() {
    // Codex restarted `item_0` on every resume; two spawns must not collide,
    // or a resumed thread's tool_result would pair against the wrong tool_use.
    let mut item = fixture_item!("observed" / "item_web_search");
    item["id"] = json!("item_0");

    let mut first = open_process();
    first.absorb(item_completed(item.clone()));
    let first_id = sources(&drain(&mut first))[0].block_json()["id"].clone();

    let mut second = open_process();
    second.absorb(item_completed(item));
    let second_id = sources(&drain(&mut second))[0].block_json()["id"].clone();

    assert_ne!(
        first_id, second_id,
        "same native item_0 on the same thread must yield collision-safe tool_use ids"
    );
}
