use super::*;

/// A thread of `turns` user turns, each a user message, a shell command whose
/// output is `output_bytes` long, and a reply, numbered as the projection
/// numbers them.
fn thread(turns: usize, output_bytes: usize) -> Value {
    let mut items = Vec::new();
    let mut messages = Vec::new();
    let mut runs = Vec::new();
    for turn in 1..=turns {
        let run = format!("t-1:{turn}");
        let ask = format!("ask-{turn}");
        let reply = format!("reply-{turn}");
        items.push(json!({
            "id": format!("item:message:{ask}"), "runId": run, "type": "user_message",
            "messageId": ask, "inputIntent": "turn_start", "createdBy": "user", "text": "Go",
        }));
        items.push(json!({
            "id": format!("item:activity:cmd-{turn}"), "runId": run, "type": "command_execution",
            "output": "x".repeat(output_bytes),
        }));
        items.push(json!({
            "id": format!("item:message:{reply}"), "runId": run, "type": "assistant_message",
            "messageId": reply, "text": "Done",
        }));
        messages.push(json!({ "id": ask, "runId": run, "text": "Go" }));
        messages.push(json!({ "id": reply, "runId": run, "text": "Done" }));
        runs.push(json!({
            "id": run, "ordinal": turn, "status": "completed",
            "userMessageId": ask,
        }));
    }
    for (ordinal, item) in items.iter_mut().enumerate() {
        item["ordinal"] = json!(ordinal);
    }
    let rows: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "position": item["ordinal"], "visibility": "local", "sourceThreadId": "t-1",
                "sourceItemId": item["id"], "item": item,
            })
        })
        .collect();
    json!({
        "thread": { "id": "t-1" },
        "runs": runs,
        "messages": messages,
        "plans": [],
        "contextHandoffs": [],
        "turnItems": items,
        "visibleTurnItems": rows,
    })
}

fn source_ids(rows: &Value) -> Vec<String> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|row| row["sourceItemId"].as_str().unwrap().to_string())
        .collect()
}

fn positions(rows: &Value) -> Vec<u64> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|row| row["position"].as_u64().unwrap())
        .collect()
}

#[test]
fn a_short_thread_is_its_own_window() {
    let whole = thread(3, 10);
    let mut projection = whole.clone();

    let window = bound_projection(&mut projection, 40);

    assert!(!window.has_more_history);
    assert_eq!(window.history_cursor, None);
    assert_eq!(window.latest_local_turn_ordinal, Some(8));
    assert_eq!(projection, whole);
}

#[test]
fn a_long_thread_keeps_its_newest_user_turns_and_their_records() {
    let whole = thread(15, 10);
    let mut projection = whole.clone();

    let window = bound_projection(&mut projection, 40);

    assert!(window.has_more_history);
    assert!(window.history_cursor.is_some());
    assert_eq!(
        window.latest_local_turn_ordinal,
        Some(44),
        "of the whole thread"
    );
    let rows = &projection["visibleTurnItems"];
    assert_eq!(
        rows.as_array().unwrap().len(),
        30,
        "ten user turns of three rows"
    );
    assert_eq!(rows[0]["sourceItemId"], "item:message:ask-6");
    assert_eq!(positions(rows), (0..30).collect::<Vec<u64>>());
    let turn_item_ids: Vec<&str> = projection["turnItems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        turn_item_ids,
        source_ids(rows),
        "a local row's item is in turnItems"
    );
    let messages = projection["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 20);
    assert_eq!(messages[0]["id"], "ask-6");
}

#[test]
fn the_window_stops_at_its_byte_budget() {
    let mut projection = thread(4, 200_000);

    let window = bound_projection(&mut projection, 40);

    // Each command row costs its output twice; two fit in a mebibyte.
    assert!(window.has_more_history);
    let rows = projection["visibleTurnItems"].as_array().unwrap();
    assert!(rows.len() < 12, "{} rows", rows.len());
    assert!(encoded_len(&projection) < PAGE_POLICY.encoded_bytes);
}

#[test]
fn older_pages_and_the_window_together_are_the_whole_timeline() {
    let whole = thread(50, 10);
    let mut window_projection = whole.clone();
    let window = bound_projection(&mut window_projection, 40);

    let mut cursor = window.history_cursor;
    let mut older: Vec<String> = Vec::new();
    let mut pages = 0;
    while let Some(encoded) = cursor {
        let page = history_page(
            &whole,
            40,
            &HistoryCursor::decode(&encoded).unwrap(),
            None,
            false,
        );
        let page_ids = source_ids(&page["items"]);
        assert_eq!(
            positions(&page["items"]),
            (0..page_ids.len() as u64).collect::<Vec<u64>>()
        );
        assert_eq!(page["hasMoreHistory"], page["nextCursor"].is_string());
        older.splice(0..0, page_ids);
        cursor = page["nextCursor"].as_str().map(str::to_string);
        pages += 1;
    }

    assert_eq!(pages, 2, "forty older turns at twenty a page");
    older.extend(source_ids(&window_projection["visibleTurnItems"]));
    assert_eq!(older, source_ids(&whole["visibleTurnItems"]));
}

#[test]
fn a_conversation_page_leaves_out_tool_rows() {
    let whole = thread(15, 10);
    let mut projection = whole.clone();
    let cursor = bound_projection(&mut projection, 40)
        .history_cursor
        .unwrap();

    let page = history_page(
        &whole,
        40,
        &HistoryCursor::decode(&cursor).unwrap(),
        None,
        true,
    );

    let ids = source_ids(&page["items"]);
    assert_eq!(ids.len(), 10);
    assert!(ids.iter().all(|id| id.starts_with("item:message:")));
}

#[test]
fn a_conversation_page_of_one_long_turn_drops_its_tool_rows_after_the_budget() {
    let mut whole = thread(1, 10);
    let mut items = Vec::new();
    for index in 0..120 {
        items.push(json!({
            "id": format!("item:activity:cmd-{index}"), "runId": "t-1:1",
            "type": "command_execution", "output": "x",
        }));
        items.push(json!({
            "id": format!("item:message:note-{index}"), "runId": "t-1:1",
            "type": "assistant_message", "text": "Note",
        }));
    }
    let rows: Vec<Value> = items
        .iter()
        .enumerate()
        .map(|(position, item)| {
            json!({
                "position": position, "visibility": "local", "sourceThreadId": "t-1",
                "sourceItemId": item["id"], "item": item,
            })
        })
        .collect();
    whole["turnItems"] = json!(items);
    whole["visibleTurnItems"] = json!(rows);
    let end = HistoryCursor {
        v: 1,
        seq: 40,
        st: "t-1".to_string(),
        si: "item:unknown".to_string(),
        p: 240,
    };

    let page = history_page(&whole, 40, &end, None, true);

    // No user turn: the row budget spans both kinds, then the tool rows go.
    let ids = source_ids(&page["items"]);
    assert_eq!(ids.len(), PAGE_POLICY.items.div_ceil(2));
    assert!(ids.iter().all(|id| id.starts_with("item:message:note-")));
    assert_eq!(
        page["items"][1]["position"], 2,
        "numbered before the filter"
    );
    assert_eq!(page["hasMoreHistory"], true);
}

#[test]
fn a_targeted_page_widens_to_reach_its_entry() {
    let whole = thread(50, 10);
    let mut projection = whole.clone();
    let cursor = bound_projection(&mut projection, 40)
        .history_cursor
        .unwrap();
    let cursor = HistoryCursor::decode(&cursor).unwrap();

    let untargeted = history_page(&whole, 40, &cursor, None, false);
    let targeted = history_page(&whole, 40, &cursor, Some("ask-5"), false);

    assert_eq!(
        untargeted["items"][0]["sourceItemId"],
        "item:message:ask-21"
    );
    assert_eq!(targeted["items"][0]["sourceItemId"], "item:message:ask-5");
}

#[test]
fn a_cursor_this_server_did_not_mint_is_refused() {
    assert_eq!(HistoryCursor::decode(""), None);
    assert_eq!(HistoryCursor::decode("not base64 json"), None);
    let wrong_version = URL_SAFE_NO_PAD.encode(r#"{"v":2,"seq":1,"st":"t","si":"i","p":0}"#);
    assert_eq!(HistoryCursor::decode(&wrong_version), None);
    assert_eq!(
        HistoryCursor::decode(&"a".repeat(CURSOR_MAX_LENGTH + 1)),
        None
    );
}

#[test]
fn a_cursor_reads_as_upstreams_encoding() {
    // `encodeThreadHistoryCursor` in `threadHistoryPaging.ts`: base64url JSON.
    let upstream = URL_SAFE_NO_PAD.encode(r#"{"v":1,"seq":40,"st":"t-1","si":"item:x","p":12}"#);

    let cursor = HistoryCursor::decode(&upstream).unwrap();

    assert_eq!(cursor.st, "t-1");
    assert_eq!(cursor.si, "item:x");
    assert_eq!(cursor.p, 12);
}
