//! The memory index a Task's system prompt carries when its Loop's LOOP.md
//! declares `memory.preload`: one summary line per entry, never a body.

use std::fmt::Write as _;

use super::models::{MemoryEntryRow, decode_scope};
use super::operations::Memories;
use super::service::visible_scopes;
use crate::task::models::Task;

/// Most entries the index lists, so a broad prefix cannot flood the prompt.
const MAX_PRELOAD_ENTRIES: usize = 100;

/// The index for `task.memory_preload`, or `None` when it declares nothing or
/// nothing matches. A failed read is logged and yields `None` rather than
/// failing the prompt.
pub async fn preload_index(memories: &Memories, task: &Task) -> Option<String> {
    if task.memory_preload.is_empty() {
        return None;
    }
    let (keys, prefixes) = split_patterns(&task.memory_preload);
    let rows = memories
        .store
        .preload(
            &visible_scopes(task),
            &keys,
            &prefixes,
            (MAX_PRELOAD_ENTRIES + 1) as i64,
        )
        .await
        .inspect_err(|e| {
            tracing::warn!(task_id = %task.task_id, error = %e, "memory preload failed");
        })
        .ok()?;
    render(&rows)
}

/// `key*` patterns become prefixes; the rest are exact keys.
fn split_patterns(patterns: &[String]) -> (Vec<String>, Vec<String>) {
    let mut keys = Vec::new();
    let mut prefixes = Vec::new();
    for pattern in patterns {
        match pattern.strip_suffix('*') {
            Some(prefix) => prefixes.push(prefix.to_string()),
            None => keys.push(pattern.clone()),
        }
    }
    (keys, prefixes)
}

fn render(rows: &[MemoryEntryRow]) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let mut out = String::from(
        "## Preloaded memory\n\nPreloaded by this loop's LOOP.md. Read an entry with \
         `moat memory get <key>` and check its citations before acting on it.\n\n",
    );
    for row in rows.iter().take(MAX_PRELOAD_ENTRIES) {
        let scope = decode_scope(row).map_or("unknown", |s| s.as_str());
        let date = row.updated_at.format("%Y-%m-%d");
        let stale = if row.stale { ", STALE" } else { "" };
        let _ = writeln!(
            out,
            "- {} ({scope}, {date}{stale}): {}",
            row.key, row.summary
        );
    }
    if rows.len() > MAX_PRELOAD_ENTRIES {
        let _ = writeln!(
            out,
            "\nMore entries match than the {MAX_PRELOAD_ENTRIES} newest listed here; \
             run `moat memory list --prefix <prefix>` for the rest."
        );
    }
    Some(out)
}

#[cfg(test)]
#[path = "preload_tests.rs"]
mod tests;
