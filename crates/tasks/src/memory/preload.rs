//! The memory index a Task's system prompt carries when its Loop's LOOP.md
//! declares `memory.preload`, and the note a resumed turn carries of what
//! changed since: one summary line per entry, never a body.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};

use super::models::{ChangedSince, MemoryEntryRow, decode_scope};
use super::operations::Memories;
use super::service::visible_scopes;
use crate::task::models::Task;

/// Most entries the index lists, so a broad prefix cannot flood the prompt.
const MAX_PRELOAD_ENTRIES: usize = 100;

/// The index for `task.memory_preload`, or `None` when it declares nothing or
/// nothing matches. A failed read is logged and yields `None` rather than
/// failing the prompt.
pub async fn preload_index(memories: &Memories, task: &Task) -> Option<String> {
    let rows = matching(memories, task, None)
        .await
        .inspect_err(|e| {
            tracing::warn!(task_id = %task.task_id, error = %e, "memory preload failed");
        })
        .ok()?;
    render(INDEX_HEADER, &rows)
}

/// What one fetch read of a task's preloaded memory.
#[derive(Debug, Default)]
pub struct MemoryRead {
    /// The time to stage if the delivery leaves the agent current.
    pub read_at: Option<DateTime<Utc>>,
    /// The entries another Task wrote since the agent last saw them, for a
    /// resumed turn: a live session keeps the index its system prompt had.
    pub note: Option<String>,
}

/// Read the task's `memory_seen` and, when `resumed`, the note. An `Err` must
/// stage nothing: staging after a failed read would skip what it missed.
pub async fn preload_changes(
    memories: &Memories,
    task: &Task,
    resumed: bool,
) -> Result<MemoryRead, sqlx::Error> {
    if task.memory_preload.is_empty() {
        return Ok(MemoryRead::default());
    }
    let seen = memories.store.seen_at(&task.task_id).await?;
    let note = match seen.seen_at.filter(|_| resumed) {
        Some(since) => {
            let changed = ChangedSince {
                since,
                reader_task_id: task.task_id.clone(),
            };
            render(
                CHANGES_HEADER,
                &matching(memories, task, Some(changed)).await?,
            )
        }
        None => None,
    };
    Ok(MemoryRead {
        read_at: Some(seen.read_at),
        note,
    })
}

/// Stage `at` as the time the agent will have seen its preloaded memory.
///
/// It counts once this fetch's delivery is acknowledged. `None` is for a
/// delivery that leaves it behind, so an earlier fetch's stage is not
/// committed by its ack. A failure is logged and the turn goes on.
pub async fn stage_seen(memories: &Memories, task_id: &str, at: Option<DateTime<Utc>>) {
    if let Err(e) = memories.store.stage_seen(task_id, at).await {
        tracing::warn!(task_id, error = %e, "memory_seen stage failed");
    }
}

/// Commit the staged time on an acknowledgement; see [`stage_seen`].
pub async fn commit_seen(memories: &Memories, task_id: &str) {
    if let Err(e) = memories.store.commit_seen(task_id).await {
        tracing::warn!(task_id, error = %e, "memory_seen commit failed");
    }
}

async fn matching(
    memories: &Memories,
    task: &Task,
    changed: Option<ChangedSince>,
) -> Result<Vec<MemoryEntryRow>, sqlx::Error> {
    if task.memory_preload.is_empty() {
        return Ok(Vec::new());
    }
    let (keys, prefixes) = split_patterns(&task.memory_preload);
    memories
        .store
        .preload(
            &visible_scopes(task),
            &keys,
            &prefixes,
            changed,
            (MAX_PRELOAD_ENTRIES + 1) as i64,
        )
        .await
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

const INDEX_HEADER: &str = "## Preloaded memory\n\nPreloaded by this loop's LOOP.md. \
     Read an entry with `moat memory get <key>` and check its citations before acting on it.\n\n";

const CHANGES_HEADER: &str = "## Memory updated since your last turn\n\nEntries this \
     loop's LOOP.md preloads that another Task wrote since your last turn. Read one with \
     `moat memory get <key>` before relying on what you knew of it.\n\n";

fn render(header: &str, rows: &[MemoryEntryRow]) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let mut out = String::from(header);
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
