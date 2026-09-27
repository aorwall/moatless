//! What a Task's lineage answers: where a cut at a turn lands, and which lines
//! of the other harness a switch session renders.
//!
//! A lineage is every event an event descends from: its chain, and across a
//! switch session's prefix the other harness's chain it was rendered from.

use std::collections::HashSet;

use serde_json::Value;
use uuid::Uuid;

use super::models::TranscodeItem;

/// One event on a lineage.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct LineageRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub harness: String,
    pub turn_number: Option<i32>,
    pub content: Value,
    /// A switch session's rendered prefix line, which no turn wrote.
    pub prefix: bool,
}

impl LineageRow {
    fn blocks(&self) -> Vec<Value> {
        self.content
            .get("blocks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    fn flag(&self, key: &str) -> bool {
        self.content.get(key).and_then(Value::as_bool) == Some(true)
    }
}

/// The event a cut after turn `at_turn` lands on: the last line a harness
/// wrote in the latest turn at or before it, moved forward past the results
/// of calls made before it. `rows` are in lineage (`ord`) order.
pub(crate) fn cut_at_turn(rows: &[LineageRow], at_turn: i32) -> Option<Uuid> {
    let own = |r: &&LineageRow| !r.prefix && r.turn_number.is_some_and(|n| n <= at_turn);
    let turn = rows
        .iter()
        .filter(own)
        .filter_map(|r| r.turn_number)
        .max()?;
    let last = rows
        .iter()
        .filter(own)
        .rfind(|r| r.turn_number == Some(turn))?;
    let session: Vec<&LineageRow> = rows
        .iter()
        .filter(|r| r.session_id == last.session_id)
        .collect();
    let at = session.iter().position(|r| r.id == last.id)?;
    Some(session[include_pending_results(&session, at)].id)
}

/// Move `at` forward to the last row carrying a `tool_result` for a
/// `tool_use` at or before `at`, when `rows` hold one.
pub(crate) fn include_pending_results(rows: &[&LineageRow], at: usize) -> usize {
    let mut pending = HashSet::new();
    for row in &rows[..=at] {
        for block in row.blocks() {
            let id = |key| block.get(key).and_then(Value::as_str).map(str::to_string);
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => pending.extend(id("id")),
                Some("tool_result") => {
                    if let Some(id) = id("tool_use_id") {
                        pending.remove(&id);
                    }
                }
                _ => {}
            }
        }
    }
    let mut cut = at;
    for (i, row) in rows.iter().enumerate().skip(at + 1) {
        if pending.is_empty() {
            break;
        }
        for block in row.blocks() {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let id = block.get("tool_use_id").and_then(Value::as_str);
            if id.is_some_and(|id| pending.remove(id)) {
                cut = i;
            }
        }
    }
    cut
}

/// The latest event of `harness` on the lineage, prefix lines included: the
/// tip a switch session into `harness` forks verbatim.
pub(crate) fn latest_of(rows: &[LineageRow], harness: &str) -> Option<Uuid> {
    rows.iter()
        .rev()
        .find(|r| r.harness == harness)
        .map(|r| r.id)
}

/// The other harness's own lines after `since`, from the last compaction on
/// (S5): what a switch session into `harness` renders after its verbatim
/// lines. Lines with no role or no block, injected ones and ones
/// `derived_from` another call are left out.
pub(crate) fn transcode_items(
    rows: &[LineageRow],
    since: Option<Uuid>,
    harness: &str,
) -> Vec<TranscodeItem> {
    let from = since
        .and_then(|since| rows.iter().position(|r| r.id == since))
        .map_or(0, |at| at + 1);
    let mut items = Vec::new();
    for row in &rows[from..] {
        if row.prefix || row.harness == harness {
            continue;
        }
        if row.flag("compaction") {
            items.clear();
        }
        let spoken = row.content.get("role").is_some_and(|r| !r.is_null());
        let derived = row
            .content
            .get("derived_from")
            .is_some_and(|d| !d.is_null());
        if !spoken || derived || row.flag("injected") || row.blocks().is_empty() {
            continue;
        }
        items.push(TranscodeItem {
            content: row.content.clone(),
            turn_number: row.turn_number,
            source_harness: row.harness.clone(),
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct Builder {
        rows: Vec<LineageRow>,
    }

    impl Builder {
        fn new() -> Self {
            Self { rows: Vec::new() }
        }

        fn line(mut self, session: u128, harness: &str, turn: i32, content: Value) -> Self {
            self.rows.push(LineageRow {
                id: Uuid::from_u128(self.rows.len() as u128 + 1),
                session_id: Uuid::from_u128(session),
                harness: harness.into(),
                turn_number: Some(turn),
                content,
                prefix: false,
            });
            self
        }

        fn text(self, session: u128, harness: &str, turn: i32, role: &str, text: &str) -> Self {
            self.line(
                session,
                harness,
                turn,
                json!({"role": role, "blocks": [{"type": "text", "text": text}]}),
            )
        }

        fn prefix(mut self) -> Self {
            self.rows.last_mut().unwrap().prefix = true;
            self
        }
    }

    fn texts(items: &[TranscodeItem]) -> Vec<&str> {
        items
            .iter()
            .map(|i| i.content["blocks"][0]["text"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn a_first_switch_transcodes_every_line_of_the_other_harness() {
        let rows = Builder::new()
            .text(1, "claude-code", 1, "user", "PERIWINKLE")
            .line(1, "claude-code", 1, json!({"role": null, "blocks": []}))
            .text(1, "claude-code", 1, "assistant", "ok")
            .rows;
        assert_eq!(latest_of(&rows, "codex"), None);
        assert_eq!(
            texts(&transcode_items(&rows, None, "codex")),
            ["PERIWINKLE", "ok"]
        );
    }

    #[test]
    fn a_switch_back_transcodes_only_the_other_lines_since() {
        // Claude turn 1, Codex turn 2 (its prefix renders turn 1), Claude again.
        let rows = Builder::new()
            .text(1, "claude-code", 1, "user", "one")
            .text(2, "codex", 2, "user", "one")
            .prefix()
            .text(2, "codex", 2, "user", "two")
            .text(2, "codex", 2, "assistant", "two done")
            .rows;
        let since = latest_of(&rows, "claude-code");
        assert_eq!(since, Some(rows[0].id));
        assert_eq!(
            texts(&transcode_items(&rows, since, "claude-code")),
            ["two", "two done"]
        );
    }

    #[test]
    fn a_compaction_starts_the_transcode_at_its_summary() {
        let rows = Builder::new()
            .text(1, "codex", 1, "user", "before")
            .line(
                1,
                "codex",
                1,
                json!({"role": "user", "blocks": [{"type": "text", "text": "summary"}], "compaction": true}),
            )
            .text(1, "codex", 2, "user", "after")
            .rows;
        assert_eq!(
            texts(&transcode_items(&rows, None, "claude-code")),
            ["summary", "after"]
        );
    }

    #[test]
    fn injected_lines_are_not_transcoded() {
        let rows = Builder::new()
            .line(
                1,
                "codex",
                1,
                json!({"role": "user", "blocks": [{"type": "text", "text": "<environment_context>"}], "injected": true}),
            )
            .text(1, "codex", 1, "user", "hi")
            .rows;
        assert_eq!(texts(&transcode_items(&rows, None, "claude-code")), ["hi"]);
    }

    #[test]
    fn a_cut_lands_on_the_last_line_of_its_turn_in_that_turns_session() {
        let rows = Builder::new()
            .text(1, "claude-code", 1, "user", "one")
            .text(1, "claude-code", 1, "assistant", "one done")
            .text(2, "codex", 2, "user", "one")
            .prefix()
            .text(2, "codex", 2, "assistant", "two done")
            .text(2, "codex", 3, "assistant", "three done")
            .rows;
        assert_eq!(cut_at_turn(&rows, 1), Some(rows[1].id));
        assert_eq!(cut_at_turn(&rows, 2), Some(rows[3].id));
        assert_eq!(cut_at_turn(&rows, 9), Some(rows[4].id));
        assert_eq!(cut_at_turn(&rows, 0), None);
    }

    #[test]
    fn a_cut_takes_the_results_of_calls_before_it() {
        let rows = Builder::new()
            .line(
                1,
                "claude-code",
                1,
                json!({"role": "assistant", "blocks": [{"type": "tool_use", "id": "t1"}]}),
            )
            .line(
                1,
                "claude-code",
                2,
                json!({"role": "user", "blocks": [{"type": "tool_result", "tool_use_id": "t1"}]}),
            )
            .rows;
        assert_eq!(cut_at_turn(&rows, 1), Some(rows[1].id));
    }
}
