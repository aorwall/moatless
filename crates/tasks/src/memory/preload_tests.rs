use std::sync::Arc;

use chrono::{TimeZone, Utc};

use super::*;
use crate::memory::dao::MockMemoryDao;
use crate::memory::models::SeenAt;
use crate::task::models::Task;
use crate::task::test_support::make_task_row;
use moatless_workspace_ports::{MockRepositoryReader, MockWorkspaceResolver};

fn row(key: &str, scope: &str, stale: bool) -> MemoryEntryRow {
    let at = Utc
        .with_ymd_and_hms(2026, 10, 1, 12, 0, 0)
        .single()
        .unwrap_or_default();
    MemoryEntryRow {
        entry_id: format!("e-{key}"),
        scope_kind: scope.to_string(),
        scope_id: "s".to_string(),
        key: key.to_string(),
        summary: format!("about {key}"),
        body: "the body is never listed".to_string(),
        data: None,
        tags: Vec::new(),
        citations: Vec::new(),
        version: 1,
        ttl_seconds: None,
        expires_at: None,
        stale_after_seconds: None,
        verified_at: None,
        claimed_by_task_id: None,
        claim_expires_at: None,
        updated_by_task_id: None,
        created_at: at,
        updated_at: at,
        stale,
    }
}

#[test]
fn patterns_ending_in_a_star_become_prefixes() {
    let (keys, prefixes) = split_patterns(&["ci:flaky".to_string(), "triage:*".to_string()]);
    assert_eq!(keys, vec!["ci:flaky"]);
    assert_eq!(prefixes, vec!["triage:"]);
}

#[test]
fn the_index_lists_summaries_and_never_bodies() {
    let index = render(
        INDEX_HEADER,
        &[
            row("ci:flaky", "loop", false),
            row("ci:old", "workspace", true),
        ],
    )
    .unwrap_or_default();
    assert!(
        index.contains("- ci:flaky (loop, 2026-10-01): about ci:flaky"),
        "{index}"
    );
    assert!(
        index.contains("- ci:old (workspace, 2026-10-01, STALE): about ci:old"),
        "{index}"
    );
    assert!(index.contains("moat memory get <key>"));
    assert!(!index.contains("the body is never listed"));
}

#[test]
fn nothing_matching_renders_no_index() {
    assert!(render(INDEX_HEADER, &[]).is_none());
}

#[test]
fn the_index_stops_at_its_cap_and_says_more_match() {
    let rows: Vec<_> = (0..=MAX_PRELOAD_ENTRIES)
        .map(|i| row(&format!("k{i}"), "loop", false))
        .collect();
    let index = render(INDEX_HEADER, &rows).unwrap_or_default();
    assert_eq!(index.matches("\n- k").count(), MAX_PRELOAD_ENTRIES);
    assert!(index.contains("More entries match"));
}

fn memories(store: MockMemoryDao) -> Memories {
    Memories::new(
        Arc::new(store),
        Arc::new(MockRepositoryReader::new()),
        Arc::new(MockWorkspaceResolver::new()),
    )
}

fn preloading_task() -> Task {
    let mut task = make_task_row("task-1", "user-1");
    task.memory_preload = vec!["ci:flaky".to_string(), "triage:*".to_string()];
    task
}

fn seen(read_at: chrono::DateTime<Utc>, seen_at: Option<chrono::DateTime<Utc>>) -> SeenAt {
    SeenAt { read_at, seen_at }
}

#[tokio::test]
async fn changes_ask_only_for_what_another_task_wrote_since_the_seen_time() {
    let since = Utc::now();
    let mut store = MockMemoryDao::new();
    store
        .expect_seen_at()
        .returning(move |_| Ok(seen(since, Some(since))));
    store
        .expect_preload()
        .withf(move |_, keys, prefixes, changed, _| {
            keys == ["ci:flaky"]
                && prefixes == ["triage:"]
                && changed
                    .as_ref()
                    .is_some_and(|c| c.since == since && c.reader_task_id == "task-1")
        })
        .returning(|_, _, _, _, _| Ok(vec![row("triage:SP-1", "loop", false)]));

    let read = preload_changes(&memories(store), &preloading_task(), true)
        .await
        .expect("read");
    let note = read.note.unwrap_or_default();
    assert!(
        note.starts_with("## Memory updated since your last turn"),
        "{note}"
    );
    assert!(note.contains("- triage:SP-1 (loop, 2026-10-01): about triage:SP-1"));
    assert_eq!(read.read_at, Some(since));
}

/// No `expect_preload`: a task whose seen time was never committed has no
/// baseline to report against, and a fresh session needs no note.
#[tokio::test]
async fn a_first_or_fresh_read_has_no_note_but_a_time_to_stage() {
    let now = Utc::now();
    for (seen_at, resumed) in [(None, true), (Some(now), false)] {
        let mut store = MockMemoryDao::new();
        store
            .expect_seen_at()
            .returning(move |_| Ok(seen(now, seen_at)));
        let read = preload_changes(&memories(store), &preloading_task(), resumed)
            .await
            .expect("read");
        assert!(read.note.is_none());
        assert_eq!(read.read_at, Some(now));
    }
}

#[tokio::test]
async fn a_failed_read_is_an_error_not_an_empty_note() {
    let now = Utc::now();
    let mut store = MockMemoryDao::new();
    store
        .expect_seen_at()
        .returning(move |_| Ok(seen(now, Some(now))));
    store
        .expect_preload()
        .returning(|_, _, _, _, _| Err(sqlx::Error::PoolTimedOut));
    assert!(
        preload_changes(&memories(store), &preloading_task(), true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_task_preloading_nothing_reads_nothing() {
    let task = make_task_row("task-1", "user-1");
    let read = preload_changes(&memories(MockMemoryDao::new()), &task, true)
        .await
        .expect("read");
    assert!(read.note.is_none() && read.read_at.is_none());
}
