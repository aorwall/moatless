//! The wire keys a shaped V2 thread snapshot carries, against the contract's
//! `OrchestrationV2ThreadBoundedSnapshot` and the thread stream's `snapshot`.

use super::*;
use crate::projection::Compactions;

// Transcribed from `packages/contracts/src/orchestrationV2.ts`
// (`soaplabs/t3code` at upstream b808f7fc6d). Every key but `idTable` is
// required: `historyCursor` and `latestLocalTurnOrdinal` as `null` included.
const BOUNDED_REQUIRED: &[&str] = &[
    "snapshotSequence",
    "projection",
    "historyCursor",
    "hasMoreHistory",
    "latestLocalTurnOrdinal",
];
const BOUNDED_OPTIONAL: &[&str] = &["turnItemsOmitLocalVisible", "ids"];
const HISTORY_KEYS: &[&str] = &["historyCursor", "hasMoreHistory", "latestLocalTurnOrdinal"];

fn detail(turns: usize) -> ThreadDetailV2 {
    let mut items = Vec::new();
    for turn in 0..turns {
        items.push(json!({
            "id": format!("item:message:ask-{turn}"), "ordinal": items.len(), "runId": null,
            "type": "user_message", "messageId": format!("ask-{turn}"),
            "inputIntent": "turn_start", "createdBy": "user", "text": "Go",
        }));
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
    ThreadDetailV2 {
        snapshot: json!({
            "snapshotSequence": 2048,
            "projection": {
                "thread": { "id": "t-1" }, "runs": [], "messages": [], "plans": [],
                "contextHandoffs": [], "checkpoints": [], "subagents": [],
                "turnItems": items, "visibleTurnItems": rows,
            },
            "historyCursor": null,
            "hasMoreHistory": false
        }),
        head: ConversationHead { position: 2 },
        lineage: projection::Lineage::root("t-1"),
    }
}

fn keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

#[test]
fn a_bounded_snapshot_carries_every_key_the_contract_requires() {
    let shape = SnapshotShape {
        bounded: true,
        compactions: Compactions {
            dedupe_bodies: true,
            compact_turn_items: true,
            compact_ids: true,
        },
    };

    let snapshot = detail(12).take_shaped(json!({}), shape);

    for key in BOUNDED_REQUIRED {
        assert!(snapshot.get(*key).is_some(), "missing {key}");
    }
    for key in keys(&snapshot) {
        assert!(
            BOUNDED_REQUIRED.contains(&key) || BOUNDED_OPTIONAL.contains(&key),
            "{key} is not in the contract"
        );
    }
    assert_eq!(snapshot["snapshotSequence"], 2048);
    assert_eq!(snapshot["hasMoreHistory"], true);
    assert!(
        snapshot["historyCursor"]
            .as_str()
            .is_some_and(|cursor| !cursor.trim().is_empty())
    );
    assert_eq!(snapshot["latestLocalTurnOrdinal"], 11);
}

#[test]
fn a_whole_stream_snapshot_carries_no_history_keys() {
    let snapshot = detail(12).take_shaped(json!({ "kind": "snapshot" }), SnapshotShape::default());

    assert_eq!(snapshot["kind"], "snapshot");
    for key in HISTORY_KEYS {
        assert!(
            snapshot.get(*key).is_none(),
            "{key} would read as a bounded window"
        );
    }
    assert_eq!(
        snapshot["projection"]["visibleTurnItems"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
}
