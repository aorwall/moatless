use super::*;

fn message_item(id: &str, text: &str) -> Value {
    json!({
        "id": format!("item:message:{id}"),
        "threadId": "t-1",
        "runId": "t-1:1",
        "type": "assistant_message",
        "messageId": id,
        "text": text,
    })
}

fn row(position: usize, item: &Value) -> Value {
    json!({
        "position": position,
        "visibility": "local",
        "sourceThreadId": "t-1",
        "sourceItemId": item["id"],
        "item": item,
    })
}

fn projection(items: &[Value], messages: Value) -> Value {
    json!({
        "thread": { "id": "t-1" },
        "messages": messages,
        "checkpoints": [],
        "subagents": [],
        "turnItems": items,
        "visibleTurnItems": items.iter().enumerate().map(|(i, item)| row(i, item)).collect::<Vec<_>>(),
    })
}

/// `expandRecordIds` from `orchestrationV2IdTable.ts`, transcribed: what the
/// client does to a response with an `ids` table.
fn expand(value: &mut Value, container: Option<&str>) {
    let ids = value["ids"].take();
    value.as_object_mut().unwrap().remove("ids");
    fn expand_record(record: &mut Map<String, Value>, ids: &Value) {
        for (key, value) in record.iter_mut() {
            if let Some(index) = value.as_u64() {
                if key == "id" || key.ends_with("Id") {
                    *value = ids[usize::try_from(index).unwrap()].clone();
                }
            } else if let Value::Array(values) = value {
                if key.ends_with("Ids") {
                    for entry in values.iter_mut() {
                        if let Some(index) = entry.as_u64() {
                            *entry = ids[usize::try_from(index).unwrap()].clone();
                        }
                    }
                }
            } else if let (true, Value::Object(item)) = (key == "item", value) {
                expand_record(item, ids);
            }
        }
    }
    let target = match container {
        Some(key) => &mut value[key],
        None => value,
    };
    for member in target.as_object_mut().unwrap().values_mut() {
        match member {
            Value::Array(records) => records
                .iter_mut()
                .filter_map(Value::as_object_mut)
                .for_each(|record| expand_record(record, &ids)),
            Value::Object(record) => expand_record(record, &ids),
            _ => {}
        }
    }
}

#[test]
fn a_message_body_its_record_carries_is_blanked_and_marked() {
    let items = [message_item("m-1", "Done."), message_item("m-2", "Edited.")];
    let mut projection = projection(
        &items,
        json!([{ "id": "m-1", "text": "Done." }, { "id": "m-2", "text": "Original." }]),
    );

    dedupe_snapshot_bodies(&mut projection);

    let first = &projection["turnItems"][0];
    assert_eq!(first["text"], "");
    assert_eq!(first["textOmitted"], true);
    assert_eq!(projection["visibleTurnItems"][0]["item"], *first);
    let second = &projection["turnItems"][1];
    assert_eq!(
        second["text"], "Edited.",
        "only a body equal to the record's is blanked"
    );
    assert!(second.get("textOmitted").is_none());
}

#[test]
fn local_turn_items_that_repeat_the_visible_rows_are_omitted() {
    let items = [message_item("m-1", "a"), message_item("m-2", "b")];
    let mut projection = projection(&items, json!([]));

    assert!(omit_local_visible_turn_items(&mut projection));
    assert_eq!(projection["turnItems"], json!([]));
    assert_eq!(projection["visibleTurnItems"][1]["item"], items[1]);
}

#[test]
fn turn_items_that_are_not_the_rows_prefix_are_kept_whole() {
    let items = [message_item("m-1", "a"), message_item("m-2", "b")];
    let mut projection = projection(&items, json!([]));
    projection["turnItems"][0]["text"] = json!("changed");
    let before = projection.clone();

    assert!(!omit_local_visible_turn_items(&mut projection));
    assert_eq!(projection, before);
}

#[test]
fn the_id_table_interns_record_ids_and_the_clients_expansion_restores_them() {
    let items = [message_item("m-1", "a"), message_item("m-2", "b")];
    let mut snapshot = json!({
        "snapshotSequence": 7,
        "projection": projection(&items, json!([{ "id": "m-1", "runId": "t-1:1", "text": "a" }])),
    });
    snapshot["projection"]["runs"] =
        json!([{ "id": "t-1:1", "handoffIds": ["h-1", 4], "ordinal": 1 }]);
    let original = snapshot.clone();

    with_id_table(&mut snapshot, Some("projection"));

    let ids = snapshot["ids"].as_array().unwrap();
    assert_eq!(
        ids.len(),
        ids.iter().collect::<std::collections::HashSet<_>>().len()
    );
    let row = &snapshot["projection"]["visibleTurnItems"][0];
    assert!(row["sourceThreadId"].is_u64() && row["item"]["messageId"].is_u64());
    assert_eq!(row["position"], 0, "a key that is not an ID is left alone");
    assert!(
        snapshot["projection"]["thread"]["id"].is_u64(),
        "a top-level object is a record"
    );
    assert_eq!(snapshot["snapshotSequence"], 7);
    expand(&mut snapshot, Some("projection"));
    assert_eq!(snapshot, original);
}

#[test]
fn an_id_table_at_the_top_level_covers_a_history_pages_rows() {
    let items = [message_item("m-1", "a")];
    let mut page = json!({
        "snapshotSequence": 3,
        "items": [row(0, &items[0])],
        "nextCursor": "abc",
        "hasMoreHistory": true,
    });
    let original = page.clone();

    with_id_table(&mut page, None);

    assert_eq!(page["nextCursor"], "abc");
    assert!(page["items"][0]["item"]["id"].is_u64());
    expand(&mut page, None);
    assert_eq!(page, original);
}

#[test]
fn compactions_apply_in_the_order_the_client_undoes_them() {
    let items = [message_item("m-1", "Done.")];
    let mut snapshot = json!({
        "snapshotSequence": 1,
        "projection": projection(&items, json!([{ "id": "m-1", "text": "Done." }])),
    });

    compact_snapshot(
        &mut snapshot,
        Compactions {
            dedupe_bodies: true,
            compact_turn_items: true,
            compact_ids: true,
        },
    );

    assert_eq!(snapshot["turnItemsOmitLocalVisible"], true);
    assert_eq!(snapshot["projection"]["turnItems"], json!([]));
    assert_eq!(
        snapshot["projection"]["visibleTurnItems"][0]["item"]["textOmitted"],
        true
    );
    assert!(snapshot["ids"].is_array());
}

#[test]
fn no_compaction_leaves_the_snapshot_as_built() {
    let items = [message_item("m-1", "Done.")];
    let mut snapshot = json!({
        "snapshotSequence": 1,
        "projection": projection(&items, json!([{ "id": "m-1", "text": "Done." }])),
    });
    let before = snapshot.clone();

    compact_snapshot(&mut snapshot, Compactions::default());

    assert_eq!(snapshot, before);
}
