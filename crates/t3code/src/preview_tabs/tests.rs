#![expect(clippy::unwrap_used, reason = "tests fail by panicking")]

use super::{PreviewTab, PreviewTabTtl, PreviewTabs};
use serde_json::json;
use std::time::Duration;

/// The two Task ids every case below works under.
const TASK: &str = "task";
const OTHER: &str = "other";

/// Wide enough that no case below can sweep its own tabs mid-test.
fn store() -> PreviewTabs {
    PreviewTabs::new(PreviewTabTtl(Duration::from_hours(1)))
}

fn tab(task_id: &str, tab_id: &str, url: &str) -> PreviewTab {
    PreviewTab {
        thread_id: task_id.to_string(),
        tab_id: tab_id.to_string(),
        nav_status: json!({ "_tag": "Success", "url": url, "title": "" }),
        can_go_back: false,
        can_go_forward: false,
        viewport: None,
        updated_at: "2026-07-31T00:00:00Z".to_string(),
    }
}

#[test]
fn opening_a_tab_lists_it() {
    let store = store();
    assert_eq!(store.list(TASK).tabs, Vec::new());
    let revision = store.put(&tab(TASK, "a", "https://a"));
    let listed = store.list(TASK);
    assert_eq!(listed.revision, revision);
    assert_eq!(listed.tabs, vec![tab(TASK, "a", "https://a")]);
}

#[test]
fn revisions_only_move_forward() {
    let store = store();
    let first = store.put(&tab(TASK, "a", "https://a"));
    let second = store.put(&tab(TASK, "b", "https://b"));
    let third = store.put(&tab(TASK, "a", "https://c"));
    let closed = store.remove(TASK, Some("a")).revision;
    assert!(first < second && second < third && third < closed);
}

#[test]
fn navigating_replaces_the_whole_tab() {
    let store = store();
    store.put(&tab(TASK, "a", "https://a"));
    store.put(&tab(TASK, "a", "https://b"));
    assert_eq!(store.list(TASK).tabs, vec![tab(TASK, "a", "https://b")]);
}

#[test]
fn closing_without_a_tab_id_closes_every_tab() {
    let store = store();
    store.put(&tab(TASK, "a", "https://a"));
    store.put(&tab(TASK, "b", "https://b"));
    store.put(&tab(OTHER, "c", "https://c"));
    let removal = store.remove(TASK, None);
    assert_eq!(removal.tab_ids, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(store.list(TASK).tabs, Vec::new());
    // Closing one Task's tabs must not reach another's.
    assert_eq!(store.list(OTHER).tabs.len(), 1);
}

#[test]
fn closing_an_unknown_tab_removes_nothing() {
    let store = store();
    store.put(&tab(TASK, "a", "https://a"));
    let removal = store.remove(TASK, Some("missing"));
    assert_eq!(removal.tab_ids, Vec::<String>::new());
    assert_eq!(store.list(TASK).tabs.len(), 1);
}

#[test]
fn a_get_answers_only_for_the_task_that_holds_the_tab() {
    let store = store();
    store.put(&tab(TASK, "a", "https://a"));
    assert_eq!(store.get(TASK, "a"), Some(tab(TASK, "a", "https://a")));
    assert_eq!(store.get(OTHER, "a"), None);
    assert_eq!(store.get(TASK, "missing"), None);
}

/// The leak `PreviewTabTtl` prevents: a replica that never sweeps holds every
/// Task it has ever served.
#[test]
fn tabs_past_their_ttl_are_swept() {
    let store = PreviewTabs::new(PreviewTabTtl(Duration::ZERO));
    store.put(&tab(TASK, "a", "https://a"));
    assert_eq!(store.list(TASK).tabs, Vec::new());
    assert_eq!(store.list(TASK).revision, 0);
}

#[tokio::test]
async fn a_published_event_reaches_a_subscriber() {
    let store = store();
    let mut events = store.subscribe();
    let published = json!({ "type": "opened" });
    store.publish(published.clone());
    assert_eq!(events.recv().await.unwrap(), published);
}

#[test]
fn the_epoch_is_stable_for_the_life_of_a_handle() {
    let first = store();
    let second = store();
    assert_eq!(first.epoch(), first.epoch());
    assert!(!first.epoch().is_empty());
    assert_ne!(first.epoch(), second.epoch());
}

#[test]
fn a_stored_tab_round_trips_through_the_wire_shape() {
    let encoded = serde_json::to_value(tab("task", "a", "https://a")).unwrap();
    assert_eq!(
        encoded,
        json!({
            "threadId": "task",
            "tabId": "a",
            "navStatus": { "_tag": "Success", "url": "https://a", "title": "" },
            "canGoBack": false,
            "canGoForward": false,
            "updatedAt": "2026-07-31T00:00:00Z"
        })
    );
}
