use chrono::{TimeZone, Utc};

use super::*;

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
    let index = render(&[
        row("ci:flaky", "loop", false),
        row("ci:old", "workspace", true),
    ])
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
    assert!(render(&[]).is_none());
}

#[test]
fn the_index_stops_at_its_cap_and_says_more_match() {
    let rows: Vec<_> = (0..=MAX_PRELOAD_ENTRIES)
        .map(|i| row(&format!("k{i}"), "loop", false))
        .collect();
    let index = render(&rows).unwrap_or_default();
    assert_eq!(index.matches("\n- k").count(), MAX_PRELOAD_ENTRIES);
    assert!(index.contains("More entries match"));
}
