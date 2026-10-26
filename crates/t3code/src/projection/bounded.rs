//! A V2 thread projection cut to its newest window, and the older pages a
//! client then asks for, as upstream T3's `threadHistoryPaging.ts` selects
//! them.
//!
//! Both are cut from the one projection a transcript builds, so a page is
//! bounded in bytes and turns, not in what the database reads.

use std::collections::HashSet;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::{Value, json};

use super::compact::is_local_row;

/// How much one window or page may hold.
///
/// Every window stops at `encoded_bytes`. One that holds a user turn also
/// stops at `user_turns` or `turn_items` rows, which keeps ordinary
/// turns whole; any other stops at `items` rows.
#[derive(Clone, Copy, Debug)]
struct PagePolicy {
    user_turns: usize,
    items: usize,
    turn_items: usize,
    encoded_bytes: usize,
}

const PAGE_POLICY: PagePolicy = PagePolicy {
    user_turns: 10,
    items: 75,
    turn_items: 500,
    encoded_bytes: 1_048_576,
};

const OLDER_USER_TURN_LIMIT: usize = 20;

const MAX_RAW_TURNS: usize = 150;

const CURSOR_MAX_LENGTH: usize = 4_096;

/// Where an older page ends: the oldest row the client already holds.
///
/// Opaque to the client, and encoded as upstream's is so a cursor reads the
/// same against either server.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct HistoryCursor {
    v: u8,
    seq: u64,
    st: String,
    si: String,
    p: usize,
}

impl HistoryCursor {
    /// `None` for anything this server did not mint; the contract answers it
    /// with `invalid_history_cursor`.
    pub(crate) fn decode(cursor: &str) -> Option<Self> {
        if cursor.is_empty() || cursor.len() > CURSOR_MAX_LENGTH {
            return None;
        }
        let bytes = URL_SAFE_NO_PAD.decode(cursor.trim_end_matches('=')).ok()?;
        let cursor: Self = serde_json::from_slice(&bytes).ok()?;
        (cursor.v == 1 && !cursor.st.is_empty() && !cursor.si.is_empty()).then_some(cursor)
    }

    /// Locates the cursor by row identity, then by `p`; `seq` is deliberately
    /// not compared, so a cursor survives the projection moving on.
    fn index_in(&self, rows: &[Value]) -> usize {
        rows.iter()
            .position(|row| row["sourceThreadId"] == *self.st && row["sourceItemId"] == *self.si)
            .unwrap_or(self.p.min(rows.len()))
    }
}

/// The fields a bounded snapshot adds beside its `projection`.
pub(crate) struct BoundedWindow {
    pub(crate) history_cursor: Option<String>,
    pub(crate) has_more_history: bool,
    pub(crate) latest_local_turn_ordinal: Option<u64>,
}

/// Cuts `projection` to its newest window in place, as
/// `buildBoundedThreadProjection` does, and says where older history resumes.
///
/// Records the client needs whatever the window holds stay: open plans, the
/// latest task list, pending handoffs, and the messages of the latest and
/// unsettled runs.
pub(crate) fn bound_projection(projection: &mut Value, snapshot_sequence: u64) -> BoundedWindow {
    let thread_id = projection["thread"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let latest_local_turn_ordinal = array(&projection["turnItems"])
        .iter()
        .filter_map(|item| item["ordinal"].as_u64())
        .max();
    let latest_todo_list = latest_todo_list(projection);
    let is_open_plan = |plan: &Value| {
        plan["status"] == "active" || plan["id"].as_str() == latest_todo_list.as_deref()
    };
    let is_open_handoff =
        |handoff: &Value| matches!(handoff["status"].as_str(), Some("pending" | "ready"));

    let control_bytes = encoded_len(&control_state(projection, &is_open_plan, &is_open_handoff));
    // Control state cannot be paged, so it may claim at most half the budget;
    // past that the window would shrink to the one row every page admits.
    let policy = PagePolicy {
        encoded_bytes: (PAGE_POLICY.encoded_bytes / 2).max(
            PAGE_POLICY
                .encoded_bytes
                .saturating_sub(control_bytes + 1_024),
        ),
        ..PAGE_POLICY
    };

    let rows = std::mem::take(&mut projection["visibleTurnItems"]);
    let rows = array(&rows);
    let start = select_older_page(rows, rows.len(), policy, |row| {
        let bytes = encoded_len(row);
        // A local row's item travels twice: in the row and in `turnItems`.
        if is_local_row(&thread_id, row) {
            bytes + encoded_len(&row["item"])
        } else {
            bytes
        }
    });
    let window = renumbered(rows[start..].to_vec());
    let window_ids: HashSet<&str> = window
        .iter()
        .filter(|row| is_local_row(&thread_id, row))
        .filter_map(|row| row["sourceItemId"].as_str())
        .collect();
    retain(&mut projection["turnItems"], |item| {
        item["id"]
            .as_str()
            .is_some_and(|id| window_ids.contains(id))
    });

    let turn_items = array(&projection["turnItems"]);
    let shown_plans = ids_of(turn_items, "planId");
    let shown_handoffs = ids_of(turn_items, "contextHandoffId");
    let mut kept_messages = ids_of(turn_items, "messageId");
    let kept_runs = kept_runs(projection);
    for run in array(&projection["runs"]) {
        let kept = run["id"].as_str().is_some_and(|id| kept_runs.contains(id));
        if let Some(message) = run["userMessageId"].as_str().filter(|_| kept) {
            kept_messages.insert(message.to_string());
        }
    }
    let shown = |ids: &HashSet<String>, record: &Value| {
        record["id"].as_str().is_some_and(|id| ids.contains(id))
    };
    retain(&mut projection["plans"], |plan| {
        is_open_plan(plan) || shown(&shown_plans, plan)
    });
    retain(&mut projection["contextHandoffs"], |handoff| {
        is_open_handoff(handoff) || shown(&shown_handoffs, handoff)
    });
    retain(&mut projection["messages"], |message| {
        shown(&kept_messages, message)
            || message["runId"]
                .as_str()
                .is_some_and(|id| kept_runs.contains(id))
            || message.get("delegatedCompletion").is_some()
            || message["workflowScript"] == true
    });
    projection["visibleTurnItems"] = Value::Array(window);
    BoundedWindow {
        history_cursor: cursor_at(rows, start, snapshot_sequence, start),
        has_more_history: start > 0,
        latest_local_turn_ordinal,
    }
}

/// The page of rows older than `cursor` in a whole projection, as the
/// contract's `OrchestrationV2ThreadHistoryPage`.
///
/// `through_entry_id` widens the page to reach that entry. `conversation_only`
/// drops tool rows from the page after numbering, and from the budget only
/// once the history has a user turn, as upstream's `getThreadHistoryPage`.
pub(crate) fn history_page(
    projection: &Value,
    snapshot_sequence: u64,
    cursor: &HistoryCursor,
    through_entry_id: Option<&str>,
    conversation_only: bool,
) -> Value {
    let rows = array(&projection["visibleTurnItems"]);
    let end = cursor.index_in(rows);
    let policy = older_page_policy(rows, end, through_entry_id);
    // Turns and rows first, bytes after: a row's cost depends on the filter.
    let start = select_older_page(rows, end, policy, |_| 0);
    let has_user_turns = rows[..end].iter().any(|row| is_user_turn(&row["item"]));
    let candidates: Vec<Value> = (start..end)
        .filter(|index| {
            !(conversation_only && has_user_turns) || is_conversation(&rows[*index]["item"])
        })
        .map(|index| {
            let mut row = rows[index].clone();
            row["position"] = json!(index);
            row
        })
        .collect();
    let bounded_start = select_older_page(&candidates, candidates.len(), policy, encoded_len);
    let (next_cursor, has_more_history) = if bounded_start > 0 {
        let canonical = candidates[bounded_start]["position"]
            .as_u64()
            .unwrap_or_default();
        let cursor = cursor_at(
            &candidates,
            bounded_start,
            snapshot_sequence,
            canonical as usize,
        );
        (cursor, true)
    } else {
        (cursor_at(rows, start, snapshot_sequence, start), start > 0)
    };
    let items: Vec<Value> = renumbered(candidates[bounded_start..].to_vec())
        .into_iter()
        .filter(|row| !conversation_only || is_conversation(&row["item"]))
        .collect();
    json!({
        "snapshotSequence": snapshot_sequence,
        "items": items,
        "nextCursor": next_cursor,
        "hasMoreHistory": has_more_history,
    })
}

/// Older pages allow twice the turns of the first window, and a targeted
/// page widens further so it reaches `through_entry_id`.
fn older_page_policy(rows: &[Value], anchor: usize, through_entry_id: Option<&str>) -> PagePolicy {
    let policy = PagePolicy {
        user_turns: OLDER_USER_TURN_LIMIT,
        ..PAGE_POLICY
    };
    let Some(target_id) = through_entry_id else {
        return policy;
    };
    let target = rows.iter().position(|row| {
        let item = &row["item"];
        item.get("messageId").unwrap_or(&item["id"]) == target_id
    });
    let Some(target) = target.filter(|target| *target < anchor) else {
        return policy;
    };
    let turns = rows[target..anchor]
        .iter()
        .filter(|row| is_user_turn(&row["item"]))
        .count();
    let reach = turns + usize::from(!is_user_turn(&rows[target]["item"]));
    PagePolicy {
        user_turns: reach.clamp(OLDER_USER_TURN_LIMIT, OLDER_USER_TURN_LIMIT * 2),
        items: policy.items * 2,
        turn_items: policy.turn_items * 2,
        ..policy
    }
}

/// Where the page of rows before `end` starts: newest first until a budget
/// is spent.
///
/// Always admits one row, so an oversized item cannot stall paging.
fn select_older_page(
    rows: &[Value],
    end: usize,
    policy: PagePolicy,
    row_cost: impl Fn(&Value) -> usize,
) -> usize {
    let end = end.min(rows.len());
    let candidates = &rows[..end];
    let turn_starts = turn_starts(candidates);
    let has_user_turn = candidates.iter().any(|row| is_user_turn(&row["item"]));
    let row_limit = if has_user_turn {
        policy.turn_items
    } else {
        policy.items
    };
    let (mut taken, mut bytes, mut user_turns, mut raw_turns) = (0, 0, 0, 0);
    for index in (0..end).rev() {
        let row = &rows[index];
        let cost = row_cost(row);
        let turn_capped =
            has_user_turn && (user_turns >= policy.user_turns || raw_turns >= MAX_RAW_TURNS);
        let full = taken >= row_limit || bytes + cost > policy.encoded_bytes || turn_capped;
        if taken > 0 && full {
            break;
        }
        taken += 1;
        bytes += cost;
        user_turns += usize::from(is_user_turn(&row["item"]));
        raw_turns += usize::from(turn_starts[index]);
    }
    // Inside a turn longer than the window no row starts a user turn, and the
    // pages after this one see only such rows and use `items`, and so does this.
    if user_turns == 0 {
        taken = taken.min(policy.items);
    }
    end - taken
}

/// The cursor resuming before `rows[start]`, whose place in the whole
/// timeline is `position`; `None` when nothing is older.
fn cursor_at(
    rows: &[Value],
    start: usize,
    snapshot_sequence: u64,
    position: usize,
) -> Option<String> {
    if start == 0 {
        return None;
    }
    let row = rows.get(start)?;
    let payload = json!({
        "v": 1,
        "seq": snapshot_sequence,
        "st": row["sourceThreadId"].as_str()?,
        "si": row["sourceItemId"].as_str()?,
        "p": position,
    });
    Some(URL_SAFE_NO_PAD.encode(payload.to_string()))
}

fn renumbered(mut rows: Vec<Value>) -> Vec<Value> {
    for (position, row) in rows.iter_mut().enumerate() {
        row["position"] = json!(position);
    }
    rows
}

fn is_turn_start(item: &Value) -> bool {
    item["type"] == "user_message"
        && matches!(
            item["inputIntent"].as_str(),
            Some("turn_start" | "queued_turn")
        )
}

fn is_user_turn(item: &Value) -> bool {
    is_turn_start(item) && item["createdBy"] == "user"
}

/// Which rows start a turn: a turn-starting user message, or a notification
/// that is its run's first input.
fn turn_starts(rows: &[Value]) -> Vec<bool> {
    let mut started: HashSet<&str> = HashSet::new();
    rows.iter()
        .map(|row| {
            let item = &row["item"];
            let notification = item["type"] == "notification";
            if item["type"] != "user_message" && !notification {
                return false;
            }
            let first_input = item["runId"]
                .as_str()
                .is_some_and(|run| started.insert(run));
            if notification {
                first_input
            } else {
                is_turn_start(item)
            }
        })
        .collect()
}

fn is_conversation(item: &Value) -> bool {
    !matches!(
        item["type"].as_str(),
        Some("command_execution" | "reasoning" | "file_change" | "dynamic_tool")
    )
}

/// The newest task list a run still claims, which outlives any window.
fn latest_todo_list(projection: &Value) -> Option<String> {
    let ordinal_of = |run_id: &str| {
        array(&projection["runs"])
            .iter()
            .find(|run| run["id"] == run_id)
            .and_then(|run| run["ordinal"].as_i64())
    };
    let mut latest: Option<(i64, String)> = None;
    for plan in array(&projection["plans"]) {
        if plan["kind"] != "todo_list" || plan["status"] == "superseded" {
            continue;
        }
        let (Some(id), Some(ordinal)) = (
            plan["id"].as_str(),
            plan["runId"].as_str().and_then(ordinal_of),
        ) else {
            continue;
        };
        if latest.as_ref().is_none_or(|(newest, _)| ordinal >= *newest) {
            latest = Some((ordinal, id.to_string()));
        }
    }
    latest.map(|(_, id)| id)
}

/// The latest run and every unsettled one: their messages stay whatever the
/// window holds.
fn kept_runs(projection: &Value) -> HashSet<String> {
    let runs = array(&projection["runs"]);
    let latest = runs
        .iter()
        .max_by_key(|run| run["ordinal"].as_i64().unwrap_or(i64::MIN));
    runs.iter()
        .filter(|run| {
            Some(*run) == latest
                || matches!(
                    run["status"].as_str(),
                    Some("preparing" | "starting" | "running" | "waiting" | "queued")
                )
        })
        .filter_map(|run| run["id"].as_str().map(str::to_string))
        .collect()
}

fn ids_of(items: &[Value], key: &str) -> HashSet<String> {
    items
        .iter()
        .filter_map(|item| item[key].as_str().map(str::to_string))
        .collect()
}

fn array(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn retain(value: &mut Value, keep: impl FnMut(&Value) -> bool) {
    if let Some(values) = value.as_array_mut() {
        values.retain(keep);
    }
}

/// The bytes `value` takes as compact JSON, without building the string.
pub(super) fn encoded_len(value: &Value) -> usize {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    let _ = serde_json::to_writer(&mut counter, value);
    counter.0
}

/// Everything a bounded projection keeps whatever its window: the
/// projection without the three pageable arrays, and with only open plans and
/// handoffs.
fn control_state(
    projection: &Value,
    is_open_plan: &impl Fn(&Value) -> bool,
    is_open_handoff: &impl Fn(&Value) -> bool,
) -> Value {
    let Some(fields) = projection.as_object() else {
        return Value::Null;
    };
    let control = fields.iter().map(|(key, value)| {
        let value = match key.as_str() {
            "messages" | "turnItems" | "visibleTurnItems" => json!([]),
            "plans" => open(value, is_open_plan),
            "contextHandoffs" => open(value, is_open_handoff),
            _ => value.clone(),
        };
        (key.clone(), value)
    });
    Value::Object(control.collect())
}

fn open(records: &Value, is_open: &impl Fn(&Value) -> bool) -> Value {
    Value::Array(
        array(records)
            .iter()
            .filter(|record| is_open(record))
            .cloned()
            .collect(),
    )
}

#[cfg(test)]
mod tests;
