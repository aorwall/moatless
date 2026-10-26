//! The opt-in wire compactions of a V2 thread snapshot: deduped bodies,
//! omitted local turn items and the ID table.
//!
//! Each is undone by the client exactly, so each must match the client's own
//! rule: `orchestrationV2DedupedBodies.ts`, `orchestrationV2BoundedSnapshot.ts`
//! and `orchestrationV2IdTable.ts` in `packages/shared` and
//! `packages/contracts`.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

/// Which compactions a client opted into for one snapshot.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Compactions {
    pub(crate) dedupe_bodies: bool,
    pub(crate) compact_turn_items: bool,
    pub(crate) compact_ids: bool,
}

/// Applies `compactions` to `snapshot`, whose `projection` holds the records,
/// in the order the client undoes them.
///
/// Dedupe runs before the turn-item omission, which compares each local row's
/// item with its `turnItems` copy and so needs both deduped alike.
pub(crate) fn compact_snapshot(snapshot: &mut Value, compactions: Compactions) {
    let Some(projection) = snapshot.get_mut("projection") else {
        return;
    };
    if compactions.dedupe_bodies {
        dedupe_snapshot_bodies(projection);
    }
    if compactions.compact_turn_items && omit_local_visible_turn_items(projection) {
        snapshot["turnItemsOmitLocalVisible"] = json!(true);
    }
    if compactions.compact_ids {
        with_id_table(snapshot, Some("projection"));
    }
}

/// Whether a visible row is this thread's own, so its item is also in
/// `turnItems` and its bodies are in this projection's records.
pub(super) fn is_local_row(thread_id: &str, row: &Value) -> bool {
    row["visibility"] == "local" || row["sourceThreadId"].as_str() == Some(thread_id)
}

/// Blanks each local turn item's body that a record in the same projection
/// carries verbatim, and marks the field, as `dedupeSnapshotBodies` does.
pub(crate) fn dedupe_snapshot_bodies(projection: &mut Value) {
    let thread_id = projection["thread"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let messages = records_by_id(&projection["messages"], "text");
    let checkpoints = records_by_id(&projection["checkpoints"], "files");
    let subagents: HashMap<String, Value> = projection["subagents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|record| Some((record["id"].as_str()?.to_string(), record.clone())))
        .collect();
    let dedupe = |item: &mut Value| dedupe_item(item, &messages, &checkpoints, &subagents);
    if let Some(rows) = projection["visibleTurnItems"].as_array_mut() {
        for row in rows.iter_mut().filter(|row| is_local_row(&thread_id, row)) {
            dedupe(&mut row["item"]);
        }
    }
    if let Some(items) = projection["turnItems"].as_array_mut() {
        items.iter_mut().for_each(dedupe);
    }
}

fn records_by_id(records: &Value, field: &str) -> HashMap<String, Value> {
    records
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|record| Some((record["id"].as_str()?.to_string(), record[field].clone())))
        .collect()
}

fn dedupe_item(
    item: &mut Value,
    messages: &HashMap<String, Value>,
    checkpoints: &HashMap<String, Value>,
    subagents: &HashMap<String, Value>,
) {
    let record = |records: &HashMap<String, Value>, key: &str| {
        item[key].as_str().and_then(|id| records.get(id)).cloned()
    };
    let kind = item["type"].as_str().map(str::to_owned);
    match kind.as_deref() {
        Some("user_message" | "assistant_message") => {
            let text = &item["text"];
            if text != "" && record(messages, "messageId").as_ref() == Some(text) {
                item["text"] = json!("");
                item["textOmitted"] = json!(true);
            }
        }
        Some("checkpoint") => {
            let files = &item["files"];
            let non_empty = files.as_array().is_some_and(|files| !files.is_empty());
            if non_empty && record(checkpoints, "checkpointId").as_ref() == Some(files) {
                item["files"] = json!([]);
                item["filesOmitted"] = json!(true);
            }
        }
        Some("subagent") => {
            let Some(previous) = record(subagents, "subagentId") else {
                return;
            };
            if item["prompt"] != "" && previous["prompt"] == item["prompt"] {
                item["prompt"] = json!("");
                item["promptOmitted"] = json!(true);
            }
            if !item["result"].is_null() && previous["result"] == item["result"] {
                item["result"] = Value::Null;
                item["resultOmitted"] = json!(true);
            }
        }
        _ => {}
    }
}

/// Drops the leading `turnItems` that repeat the local visible rows' items,
/// as `omitLocalVisibleTurnItems` does; `false`, with nothing dropped, when
/// they are not an exact prefix.
pub(crate) fn omit_local_visible_turn_items(projection: &mut Value) -> bool {
    let thread_id = projection["thread"]["id"].as_str().unwrap_or_default();
    let (Some(rows), Some(items)) = (
        projection["visibleTurnItems"].as_array(),
        projection["turnItems"].as_array(),
    ) else {
        return false;
    };
    let mut omitted = 0;
    for row in rows.iter().filter(|row| is_local_row(thread_id, row)) {
        if items.get(omitted) != Some(&row["item"]) {
            return false;
        }
        omitted += 1;
    }
    if omitted == 0 {
        return false;
    }
    if let Some(items) = projection["turnItems"].as_array_mut() {
        items.drain(..omitted);
    }
    true
}

/// Replaces each record's ID values with indexes into an `ids` table added
/// to `value`, under `orchestrationV2IdTable.ts`'s rule.
///
/// The records sit under `container`, or at `value`'s top level when it is
/// `None`. A number under an ID key is always read back as an index, so every
/// string or number there is interned, never only the repeated ones.
pub(crate) fn with_id_table(value: &mut Value, container: Option<&str>) {
    let mut table = IdTable::default();
    let target = match container {
        Some(key) => value.get_mut(key),
        None => Some(&mut *value),
    };
    let Some(Value::Object(target)) = target else {
        return;
    };
    for member in target.values_mut() {
        match member {
            Value::Array(records) => records
                .iter_mut()
                .filter_map(Value::as_object_mut)
                .for_each(|record| table.intern_record(record)),
            Value::Object(record) => table.intern_record(record),
            _ => {}
        }
    }
    value["ids"] = Value::Array(table.ids);
}

#[derive(Default)]
struct IdTable {
    ids: Vec<Value>,
    index: HashMap<IdKey, usize>,
}

/// A JS `Map` key: the string `"1"` and the number `1` are distinct entries.
#[derive(Hash, PartialEq, Eq)]
enum IdKey {
    String(String),
    Number(String),
}

impl IdTable {
    fn intern_record(&mut self, record: &mut Map<String, Value>) {
        for (key, value) in record.iter_mut() {
            if key == "id" || key.ends_with("Id") {
                self.intern(value);
            } else if key.ends_with("Ids") {
                if let Value::Array(values) = value {
                    values.iter_mut().for_each(|value| self.intern(value));
                }
            } else if key == "item" {
                if let Value::Object(item) = value {
                    self.intern_record(item);
                }
            }
        }
    }

    fn intern(&mut self, value: &mut Value) {
        let key = match value {
            Value::String(id) => IdKey::String(id.clone()),
            Value::Number(id) => IdKey::Number(id.to_string()),
            _ => return,
        };
        let next = self.ids.len();
        let position = *self.index.entry(key).or_insert(next);
        if position == next {
            self.ids.push(value.take());
        }
        *value = json!(position);
    }
}

#[cfg(test)]
mod tests;
