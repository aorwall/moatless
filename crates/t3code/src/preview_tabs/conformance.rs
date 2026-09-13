#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::contracts::{PreviewTab, PreviewTabDao};
use serde_json::json;
use std::time::Duration;

/// The two Task ids every case below works under.
///
/// Fixed rather than generated, so a caller sharing a store with anything else
/// — as the integration tier does, against a real Redis — owns them for the
/// run.
pub const TASK: &str = "task";
pub const OTHER: &str = "other";

pub fn tab(task_id: &str, tab_id: &str, url: &str) -> PreviewTab {
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

/// Start from no tabs.
///
/// `run_every_case` runs every case against one handle; a persisting store (the
/// property the Redis one exists for) would otherwise carry each case's tabs
/// into the next.
async fn reset(store: &dyn PreviewTabDao) {
    store.remove(TASK, None).await.unwrap();
    store.remove(OTHER, None).await.unwrap();
}

pub async fn opening_a_tab_lists_it(store: &dyn PreviewTabDao) {
    reset(store).await;
    assert_eq!(store.list(TASK).await.unwrap().tabs, Vec::new());
    let revision = store.put(&tab(TASK, "a", "https://a")).await.unwrap();
    let listed = store.list(TASK).await.unwrap();
    assert_eq!(listed.revision, revision);
    assert_eq!(listed.tabs, vec![tab(TASK, "a", "https://a")]);
}

pub async fn revisions_only_move_forward(store: &dyn PreviewTabDao) {
    reset(store).await;
    let first = store.put(&tab(TASK, "a", "https://a")).await.unwrap();
    let second = store.put(&tab(TASK, "b", "https://b")).await.unwrap();
    let third = store.put(&tab(TASK, "a", "https://c")).await.unwrap();
    let closed = store.remove(TASK, Some("a")).await.unwrap().revision;
    assert!(first < second && second < third && third < closed);
}

pub async fn navigating_replaces_the_whole_tab(store: &dyn PreviewTabDao) {
    reset(store).await;
    store.put(&tab(TASK, "a", "https://a")).await.unwrap();
    store.put(&tab(TASK, "a", "https://b")).await.unwrap();
    let listed = store.list(TASK).await.unwrap();
    assert_eq!(listed.tabs, vec![tab(TASK, "a", "https://b")]);
}

pub async fn closing_without_a_tab_id_closes_every_tab(store: &dyn PreviewTabDao) {
    reset(store).await;
    store.put(&tab(TASK, "a", "https://a")).await.unwrap();
    store.put(&tab(TASK, "b", "https://b")).await.unwrap();
    store.put(&tab(OTHER, "c", "https://c")).await.unwrap();
    let removal = store.remove(TASK, None).await.unwrap();
    assert_eq!(removal.tab_ids, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(store.list(TASK).await.unwrap().tabs, Vec::new());
    // Closing one Task's tabs must not reach another's.
    assert_eq!(store.list(OTHER).await.unwrap().tabs.len(), 1);
}

pub async fn closing_an_unknown_tab_removes_nothing(store: &dyn PreviewTabDao) {
    reset(store).await;
    store.put(&tab(TASK, "a", "https://a")).await.unwrap();
    let removal = store.remove(TASK, Some("missing")).await.unwrap();
    assert_eq!(removal.tab_ids, Vec::<String>::new());
    assert_eq!(store.list(TASK).await.unwrap().tabs.len(), 1);
}

pub async fn a_published_event_reaches_a_subscriber(store: &dyn PreviewTabDao) {
    let mut events = store.subscribe();
    let published = json!({ "type": "opened" });
    // Two things make a single publish-then-receive the wrong assertion
    // against Redis. Its pub/sub reaches whoever is subscribed at that
    // instant and nobody else, and a handle's forwarder subscribes
    // asynchronously after `connect` has already returned — so an early
    // publish is simply dropped. And the channel is shared with whatever
    // else points at that database, so the next event to arrive is not
    // necessarily this one. Republishing until this event lands asserts
    // that the channel reaches this subscriber, and nothing about when.
    let delivered = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            store.publish(published.clone()).await.unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
            while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, events.recv()).await {
                if event == published {
                    return;
                }
            }
        }
    })
    .await;
    assert!(
        delivered.is_ok(),
        "a published event should reach a subscriber on the same handle"
    );
}

/// Every case above, in one call, for a caller that wants "does this handle
/// behave"; each case resets first, so they compose.
///
/// Reached only from `redis_tests` — the unit tier names each case one at a
/// time.
#[cfg(feature = "db-tests")]
pub async fn run_every_case(store: &dyn PreviewTabDao) {
    opening_a_tab_lists_it(store).await;
    revisions_only_move_forward(store).await;
    navigating_replaces_the_whole_tab(store).await;
    closing_without_a_tab_id_closes_every_tab(store).await;
    closing_an_unknown_tab_removes_nothing(store).await;
    a_published_event_reaches_a_subscriber(store).await;
    reset(store).await;
}
